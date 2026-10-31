//! The MCP endpoint through the gateway: two principals call `POST /mcp` over real sockets. The
//! expectations come from `docs/mcp-v0.md`, not from running the server. No assertion depends on
//! time; the only waits are socket timeouts that a reply never reaches.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_e2e::json::Json;
use lattice_e2e::mcp::{Mcp, Reply};
use lattice_e2e::{Fixture, kill_points};

/// The served workspace: the only scope the endpoint accepts (docs/mcp-v0.md, HTTP transport).
const SCOPE: &str = "default";

/// The cursor before the first event: twenty-four zeros (docs/mcp-v0.md, Subscribe, Cursor).
const ORIGIN: &str = "000000000000000000000000";

/// The most events one `lattice_subscribe` reply carries (docs/mcp-v0.md, Subscribe, Bounds).
const PAGE: usize = 64;

/// The `result` member of a JSON-RPC reply that succeeded.
fn result(reply: &Reply) -> Json {
    assert_eq!(reply.status, 200, "{}", reply.head);
    let json = reply.json().unwrap();
    json.get("result")
        .cloned()
        .unwrap_or_else(|| panic!("no result in {}", reply.body))
}

/// The code and message of a JSON-RPC error reply.
fn rpc_error(reply: &Reply) -> (i64, String) {
    assert_eq!(reply.status, 200, "{}", reply.head);
    let json = reply.json().unwrap();
    let error = json
        .get("error")
        .unwrap_or_else(|| panic!("no error in {}", reply.body));
    let code = error.get("code").and_then(Json::as_i64).unwrap();
    let message = error.get("message").and_then(Json::as_str).unwrap();
    (code, message.to_owned())
}

/// A `tools/call` result: the text of its one content block, and whether the tool failed.
struct Tool {
    is_error: bool,
    text: String,
}

fn tool(reply: &Reply) -> Tool {
    let result = result(reply);
    let blocks = result.get("content").and_then(Json::as_array).unwrap();
    assert_eq!(blocks.len(), 1, "{}", reply.body);
    assert_eq!(blocks[0].get("type").and_then(Json::as_str), Some("text"));
    Tool {
        is_error: result.get("isError").and_then(Json::as_bool).unwrap(),
        text: blocks[0]
            .get("text")
            .and_then(Json::as_str)
            .unwrap()
            .to_owned(),
    }
}

/// The tool names in a `tools/list` reply, in the order the reply gives them.
fn tool_names(reply: &Reply) -> Vec<String> {
    let result = result(reply);
    let tools = result.get("tools").and_then(Json::as_array).unwrap();
    tools
        .iter()
        .map(|t| t.get("name").and_then(Json::as_str).unwrap().to_owned())
        .collect()
}

/// A cursor is 24 lowercase hex digits (docs/mcp-v0.md, Subscribe, Cursor).
fn is_cursor(text: &str) -> bool {
    text.len() == 24
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One event of the feed as a reply shows it: its cursor and, for a row write, the row's key.
#[derive(Debug, Clone, PartialEq)]
struct Event {
    cursor: String,
    key: Option<String>,
}

/// One `lattice_subscribe` reply.
#[derive(Debug)]
struct Page {
    events: Vec<Event>,
    truncated: bool,
    next: String,
}

/// One `lattice_subscribe` call from `after` (the origin when `None`), checked against the rules
/// of docs/mcp-v0.md that a single reply must meet.
fn subscribe(mcp: &Mcp, after: Option<&str>) -> Page {
    let arguments = match after {
        Some(cursor) => format!(r#"{{"scope":"{SCOPE}","table":"kv","after":"{cursor}"}}"#),
        None => format!(r#"{{"scope":"{SCOPE}","table":"kv"}}"#),
    };
    let reply = tool(&mcp.call(1, "lattice_subscribe", &arguments).unwrap());
    assert!(!reply.is_error, "{}", reply.text);
    let envelope = Json::parse(&reply.text).unwrap();
    assert_eq!(
        envelope.get("tool").and_then(Json::as_str),
        Some("lattice_subscribe")
    );
    let next = envelope
        .get("next_cursor")
        .and_then(Json::as_str)
        .unwrap()
        .to_owned();
    let truncated = envelope.get("truncated").and_then(Json::as_bool).unwrap();
    let items = envelope
        .get("untrusted")
        .and_then(|u| u.get("events"))
        .and_then(Json::as_array)
        .unwrap();
    let events: Vec<Event> = items
        .iter()
        .map(|item| Event {
            cursor: item
                .get("cursor")
                .and_then(Json::as_str)
                .unwrap()
                .to_owned(),
            // A row write carries its columns; a commit without a row carries none and has no key.
            key: item
                .get("columns")
                .and_then(|c| c.get("key"))
                .and_then(Json::as_str)
                .map(str::to_owned),
        })
        .collect();

    assert!(is_cursor(&next), "{next}");
    assert!(events.iter().all(|e| is_cursor(&e.cursor)));
    assert!(events.len() <= PAGE, "a reply holds at most {PAGE} events");
    // A reply stops early only at its event bound, and then says that more events exist.
    if truncated {
        assert_eq!(events.len(), PAGE, "{}", reply.text);
    }
    // next_cursor is the cursor of the last event returned, or the cursor given when none was.
    let given = after.unwrap_or(ORIGIN);
    let expected = events.last().map_or(given, |e| e.cursor.as_str());
    assert_eq!(next, expected);
    Page {
        events,
        truncated,
        next,
    }
}

/// Reads pages from `from` (the origin when `None`) until one comes back empty, and returns every
/// page, the empty one last. Cursors strictly increase, so no event is repeated, and each event
/// comes after the cursor it was read from.
fn drain(mcp: &Mcp, from: Option<&str>) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut after: Option<String> = from.map(str::to_owned);
    loop {
        let page = subscribe(mcp, after.as_deref());
        let mut last: String = after.clone().unwrap_or_else(|| ORIGIN.to_owned());
        for event in &page.events {
            assert!(
                event.cursor > last,
                "{} does not come after {last}",
                event.cursor
            );
            last = event.cursor.clone();
        }
        let empty = page.events.is_empty();
        after = Some(page.next.clone());
        pages.push(page);
        if empty {
            return pages;
        }
    }
}

/// The keys of the row writes in `pages`, in feed order.
fn keys(pages: &[Page]) -> Vec<String> {
    pages
        .iter()
        .flat_map(|p| p.events.iter())
        .filter_map(|e| e.key.clone())
        .collect()
}

/// The cursors of the row writes in `pages`, in feed order.
fn keyed_cursors(pages: &[Page]) -> Vec<String> {
    pages
        .iter()
        .flat_map(|p| p.events.iter())
        .filter(|e| e.key.is_some())
        .map(|e| e.cursor.clone())
        .collect()
}

#[test]
fn two_principals_write_and_read_through_the_gateway() {
    let fixture = Fixture::start("mcp-principals").unwrap();
    fixture.create_kv().unwrap();
    let alice = fixture.mcp(Some(fixture.tokens.alice.as_str()));
    let bob = fixture.mcp(Some(fixture.tokens.bob.as_str()));

    // A client that opens with the 2025-11-25 handshake gets that version back, with no session.
    let init = alice
        .post(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#)
        .unwrap();
    assert_eq!(
        result(&init).get("protocolVersion").and_then(Json::as_str),
        Some("2025-11-25")
    );

    // Tools are listed only where the token holds a grant. Alice may write; bob may only read.
    assert_eq!(
        tool_names(&alice.list(2).unwrap()),
        ["lattice_read", "lattice_write", "lattice_subscribe"]
    );
    assert_eq!(
        tool_names(&bob.list(2).unwrap()),
        ["lattice_read", "lattice_subscribe"]
    );

    // Alice writes a row. The write reply has a fixed shape and never repeats the value.
    let written = tool(
        &alice
            .call(
                3,
                "lattice_write",
                r#"{"scope":"default","table":"kv","key":"k1","value":"v1"}"#,
            )
            .unwrap(),
    );
    assert!(!written.is_error);
    assert_eq!(
        written.text,
        r#"{"tool":"lattice_write","applied":true,"version":1}"#
    );

    // Bob reads the row alice wrote: one row in the fixed envelope, at version 1.
    let read = tool(
        &bob.call(
            4,
            "lattice_read",
            r#"{"scope":"default","table":"kv","key":"k1"}"#,
        )
        .unwrap(),
    );
    assert!(!read.is_error);
    let envelope = Json::parse(&read.text).unwrap();
    assert_eq!(
        envelope.get("tool").and_then(Json::as_str),
        Some("lattice_read")
    );
    assert_eq!(
        envelope.get("truncated").and_then(Json::as_bool),
        Some(false)
    );
    let rows = envelope
        .get("untrusted")
        .and_then(|u| u.get("rows"))
        .and_then(Json::as_array)
        .unwrap();
    assert_eq!(rows.len(), 1, "{}", read.text);
    assert_eq!(rows[0].get("key").and_then(Json::as_str), Some("k1"));
    assert_eq!(rows[0].get("value").and_then(Json::as_str), Some("v1"));
    assert_eq!(rows[0].get("version").and_then(Json::as_str), Some("1"));

    // A row that does not exist is the same answer as a table that does not exist.
    let missing = tool(
        &bob.call(
            5,
            "lattice_read",
            r#"{"scope":"default","table":"kv","key":"ghost"}"#,
        )
        .unwrap(),
    );
    assert!(missing.is_error);
    assert_eq!(missing.text, r#"{"error":"not_found"}"#);

    // Bob holds no write grant, so the write tool is not granted to him at all.
    let (code, message) = rpc_error(
        &bob.call(
            6,
            "lattice_write",
            r#"{"scope":"default","table":"kv","key":"k2","value":"x"}"#,
        )
        .unwrap(),
    );
    assert_eq!(code, 40301);
    assert_eq!(message, "tool not granted");
    let absent = tool(
        &alice
            .call(
                7,
                "lattice_read",
                r#"{"scope":"default","table":"kv","key":"k2"}"#,
            )
            .unwrap(),
    );
    assert_eq!(
        absent.text, r#"{"error":"not_found"}"#,
        "a refused write changed nothing"
    );

    // A scope other than the served workspace is refused, and the refusal names the served scope.
    let (code, message) = rpc_error(
        &alice
            .call(
                8,
                "lattice_write",
                r#"{"scope":"elsewhere","table":"kv","key":"k3","value":"x"}"#,
            )
            .unwrap(),
    );
    assert_eq!(code, 40302);
    assert_eq!(
        message,
        r#"scope denied; this server serves scope "default""#
    );
    // The refused write left no row behind: k3 does not exist in the served scope.
    let k3 = tool(
        &alice
            .call(
                13,
                "lattice_read",
                r#"{"scope":"default","table":"kv","key":"k3"}"#,
            )
            .unwrap(),
    );
    assert_eq!(
        k3.text, r#"{"error":"not_found"}"#,
        "a refused write changed nothing"
    );

    // bob holds a read grant, so the same refusal reaches him: 40302 names the served scope too.
    let (code, message) = rpc_error(
        &bob.call(
            14,
            "lattice_read",
            r#"{"scope":"elsewhere","table":"kv","key":"k1"}"#,
        )
        .unwrap(),
    );
    assert_eq!(code, 40302);
    assert_eq!(
        message,
        r#"scope denied; this server serves scope "default""#
    );

    // expected_version 0 says the row must not exist; k1 is at version 1, so the write conflicts and
    // the stored row is unchanged.
    let conflict = tool(
        &alice
            .call(
                9,
                "lattice_write",
                r#"{"scope":"default","table":"kv","key":"k1","value":"v9","expected_version":0}"#,
            )
            .unwrap(),
    );
    assert!(conflict.is_error);
    assert!(
        conflict.text.contains("version conflict"),
        "{}",
        conflict.text
    );

    // expected_version 1 matches, so the write applies and the row moves to version 2.
    let second = tool(
        &alice
            .call(
                10,
                "lattice_write",
                r#"{"scope":"default","table":"kv","key":"k1","value":"v2","expected_version":1}"#,
            )
            .unwrap(),
    );
    assert_eq!(
        second.text,
        r#"{"tool":"lattice_write","applied":true,"version":2}"#
    );
    let reread = tool(
        &bob.call(
            11,
            "lattice_read",
            r#"{"scope":"default","table":"kv","key":"k1"}"#,
        )
        .unwrap(),
    );
    assert!(reread.text.contains(r#""value":"v2""#), "{}", reread.text);
    assert!(reread.text.contains(r#""version":"2""#), "{}", reread.text);

    // No credential, and an unknown one, get the same 401, byte for byte.
    let anonymous = Mcp::new(fixture.ws, None).list(12).unwrap();
    assert_eq!(anonymous.status, 401, "{}", anonymous.head);
    assert_eq!(anonymous.body, "authentication required");
    assert_eq!(
        anonymous.header("www-authenticate").as_deref(),
        Some("Bearer")
    );
    // A well-formed token the gateway never issued, built at run time.
    let unknown_token = format!("lat_{}", "0".repeat(56));
    let unknown = Mcp::new(fixture.ws, Some(unknown_token.as_str()))
        .list(12)
        .unwrap();
    assert_eq!(unknown.status, 401, "{}", unknown.head);
    assert_eq!(unknown.body, anonymous.body);
    assert_eq!(unknown.head, anonymous.head);
}

#[test]
fn a_cursor_chain_over_a_seeded_history_has_no_gap_and_no_repeat() {
    let fixture = Fixture::start("mcp-cursor-chain").unwrap();
    fixture.create_kv().unwrap();
    let alice = fixture.mcp(Some(fixture.tokens.alice.as_str()));
    let bob = fixture.mcp(Some(fixture.tokens.bob.as_str()));

    // Set aside whatever the feed holds before the first write; the chain starts after it.
    let before_any_write = drain(&alice, None);
    let mut chain: String = before_any_write.last().unwrap().next.clone();

    // A first batch longer than one page, then batches whose sizes come from a seeded schedule:
    // the gap between consecutive points of `kill_points(11, 8)`.
    let mut sizes: Vec<u64> = vec![150];
    let mut previous = 0;
    for point in kill_points(11, 8) {
        sizes.push(point - previous);
        previous = point;
    }

    let mut written: Vec<String> = Vec::new();
    let mut chained: Vec<Page> = Vec::new();
    for (batch, size) in sizes.into_iter().enumerate() {
        let from = written.len();
        for _ in 0..size {
            let n = written.len() + 1;
            let key = format!("k{n:05}");
            let value = format!("v{n:05}");
            let arguments =
                format!(r#"{{"scope":"{SCOPE}","table":"kv","key":"{key}","value":"{value}"}}"#);
            let reply = tool(&alice.call(1, "lattice_write", &arguments).unwrap());
            assert!(!reply.is_error, "{}", reply.text);
            written.push(key);
        }

        let pages = drain(&alice, Some(&chain));
        assert_eq!(keys(&pages), written[from..], "batch {batch}");
        if batch == 0 {
            // 150 new events: two full pages that say more follow, then 22, then the empty page.
            let shape: Vec<(usize, bool)> = pages
                .iter()
                .map(|p| (p.events.len(), p.truncated))
                .collect();
            assert_eq!(shape, [(64, true), (64, true), (22, false), (0, false)]);
        }
        chain = pages.last().unwrap().next.clone();
        chained.extend(pages);
    }

    // The whole chain, in order, is every write exactly once.
    assert_eq!(keys(&chained), written);
    assert!(written.len() > PAGE);

    // Bob reads the same feed from the origin. He sees the same row writes under the same cursors.
    let bob_pages = drain(&bob, None);
    assert_eq!(keys(&bob_pages), written);
    assert_eq!(keyed_cursors(&bob_pages), keyed_cursors(&chained));

    // Resuming from the cursor of the 100th write gives exactly the writes after it.
    let cursors = keyed_cursors(&chained);
    let middle = cursors[99].clone();
    let tail = drain(&bob, Some(&middle));
    assert_eq!(keys(&tail), written[100..]);
}
