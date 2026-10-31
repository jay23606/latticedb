//! The MCP endpoint over real sockets: `POST /mcp` on the gateway port (issue #527, part 1) with the
//! browser and DNS-rebinding defences of issue #580. Requests are raw HTTP written here, and every
//! response is judged from its bytes, not from the server's own types.
//!
//! The expectations come from `docs/mcp-v0.md` and the wire rules, not from running the code under
//! test. Two principals (`alice`, who may write `kv`, and `bob`, who may only read it) prove that the
//! principal comes from the verified token.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use lattice_api::bearer::{CredentialStore, TokenId, issue, verify_bearer};
use lattice_api::exec_host::{ExecHost, Grant, HostLimits, PolicyTable};
use lattice_api::principal::Principal;
use lattice_seams::OsRng;
use latticed::ws::{AllowedHost, GatewayAuth, GatewayConfig};
use latticed::{ServerConfig, SqlConfig, SqlServer, load_credential_store, write_credential_store};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `alice` reads and writes `kv`; `bob` reads `kv` only; `carol` holds `other` only; `admin` is
/// allowed everything (used to create the tables); `dave` is denied everything.
const ACCOUNTS: &str = "admin\tallow-all\nalice\ttables: kv=rw\nbob\ttables: kv=r\ncarol\ttables: other=rw\ndave\tdeny-all\n";
const KV: &str = "CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT, version BIGINT NOT NULL)";
const OTHER: &str =
    "CREATE TABLE other (key TEXT PRIMARY KEY, value TEXT, version BIGINT NOT NULL)";
const NAMES: [&str; 5] = ["admin", "alice", "bob", "carol", "dave"];

const META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

struct Fixture {
    server: Option<SqlServer>,
    port: u16,
    tokens: BTreeMap<&'static str, String>,
    dir: PathBuf,
    creds: Option<PathBuf>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.shutdown();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
        if let Some(creds) = &self.creds {
            let _ = std::fs::remove_dir_all(creds);
        }
    }
}

impl Fixture {
    fn token(&self, name: &str) -> String {
        self.tokens[name].clone()
    }
}

fn private_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .unwrap();
}

fn tokens_and_store() -> (CredentialStore, BTreeMap<&'static str, String>) {
    let mut store = CredentialStore::empty();
    let mut tokens = BTreeMap::new();
    for name in NAMES {
        let (token, entry) = issue(name, 1, None, &mut OsRng).unwrap();
        store.insert(entry).unwrap();
        tokens.insert(name, token.expose_secret().to_owned());
    }
    (store, tokens)
}

fn policies() -> PolicyTable {
    PolicyTable::new(Grant::DenyAll).with_accounts(PolicyTable::parse_accounts(ACCOUNTS).unwrap())
}

/// Creates the two tables with the executor, then closes it, so the server can open the directory.
fn seed(dir: &Path, store: &CredentialStore, tokens: &BTreeMap<&'static str, String>) {
    let host = ExecHost::start_dir(dir.to_path_buf(), policies(), HostLimits::default()).unwrap();
    let admin = principal(store, &tokens["admin"]);
    for ddl in [KV, OTHER] {
        let mut sql = host.sql_for(admin.clone());
        sql.run(ddl, &[]).unwrap();
    }
    host.shutdown().unwrap();
}

fn principal(store: &CredentialStore, token: &str) -> Principal {
    let header = format!("Bearer {token}");
    verify_bearer(Some(header.as_bytes()), store, unix_now()).unwrap()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn fast(config: &mut GatewayConfig) {
    config.address = SocketAddr::from(([127, 0, 0, 1], 0));
    config.slice = Duration::from_millis(10);
    config.failure_limit = 100;
}

/// A server whose tokens are given in memory.
fn start(name: &str, tune: impl FnOnce(&mut GatewayConfig)) -> Fixture {
    let dir = std::env::temp_dir().join(format!("latticed-mcp-http-{name}-{}", std::process::id()));
    private_dir(&dir);
    let (store, tokens) = tokens_and_store();
    seed(&dir, &store, &tokens);
    let mut gateway = GatewayConfig {
        auth: GatewayAuth::Bearer(store),
        ..GatewayConfig::default()
    };
    fast(&mut gateway);
    tune(&mut gateway);
    let server = boot(&dir, gateway);
    Fixture {
        port: server.gateway_addr().unwrap().port(),
        server: Some(server),
        tokens,
        dir,
        creds: None,
    }
}

/// A server whose tokens come from a credential file the test edits.
fn start_file(name: &str, tune: impl FnOnce(&mut GatewayConfig)) -> (Fixture, PathBuf) {
    let dir = std::env::temp_dir().join(format!("latticed-mcp-http-{name}-{}", std::process::id()));
    private_dir(&dir);
    let (store, tokens) = tokens_and_store();
    seed(&dir, &store, &tokens);
    let creds = std::env::temp_dir().join(format!(
        "latticed-mcp-http-{name}-creds-{}",
        std::process::id()
    ));
    private_dir(&creds);
    let file = creds.join("bearer-tokens");
    write_credential_store(&file, &store).unwrap();
    let mut gateway = GatewayConfig {
        auth: GatewayAuth::BearerFile(file.clone()),
        credential_poll: Duration::from_millis(50),
        ..GatewayConfig::default()
    };
    fast(&mut gateway);
    tune(&mut gateway);
    let server = boot(&dir, gateway);
    let fixture = Fixture {
        port: server.gateway_addr().unwrap().port(),
        server: Some(server),
        tokens,
        dir,
        creds: Some(creds),
    };
    (fixture, file)
}

fn boot(dir: &Path, gateway: GatewayConfig) -> SqlServer {
    SqlServer::start_with_gateway(
        ServerConfig {
            address: SocketAddr::from(([127, 0, 0, 1], 0)),
            ..ServerConfig::default()
        },
        SqlConfig {
            data_dir: dir.to_path_buf(),
            policies: policies(),
            limits: HostLimits::default(),
            allow_format_upgrade: false,
        },
        Some(gateway),
    )
    .unwrap()
}

// --- raw HTTP ------------------------------------------------------------------------------------

/// One response: the status, the head (lines up to the blank line) and the body as text.
struct Resp {
    status: u16,
    head: String,
    body: String,
}

impl Resp {
    /// The value of a response header, matched case-insensitively.
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (n, v) = line.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_owned())
        })
    }
}

/// Writes `bytes` on a new connection and reads until the server closes it.
fn exchange(port: u16, bytes: &[u8]) -> Resp {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // A refused request may close before every byte was sent; the answer still counts.
    let _ = socket.write_all(bytes);
    let mut raw = Vec::new();
    let _ = socket.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = match text.split_once("\r\n\r\n") {
        Some((h, b)) => (h.to_owned(), b.to_owned()),
        None => (text.clone(), String::new()),
    };
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Resp { status, head, body }
}

fn raw(method: &str, target: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
    let mut out = format!("{method} {target} HTTP/1.1\r\n");
    for h in headers {
        out.push_str(h);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// The headers of a well-formed `POST /mcp` for `port`, with `auth` as its bearer token.
fn headers(port: u16, auth: Option<&str>, length: usize) -> Vec<String> {
    let mut h = vec![
        format!("Host: localhost:{port}"),
        "Content-Type: application/json".to_owned(),
        format!("Content-Length: {length}"),
    ];
    if let Some(token) = auth {
        h.push(format!("Authorization: Bearer {token}"));
    }
    h
}

/// Sends one JSON-RPC message to `/mcp` as `auth`.
fn call(port: u16, auth: Option<&str>, body: &str) -> Resp {
    exchange(
        port,
        &raw(
            "POST",
            "/mcp",
            &headers(port, auth, body.len()),
            body.as_bytes(),
        ),
    )
}

fn list(id: u32) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list","params":{{{META}}}}}"#)
}

fn tool(id: u32, name: &str, arguments: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{{META},"name":"{name}","arguments":{arguments}}}}}"#
    )
}

fn write_args(table: &str, key: &str, value: &str) -> String {
    format!(r#"{{"scope":"default","table":"{table}","key":"{key}","value":"{value}"}}"#)
}

fn read_args(table: &str, key: &str) -> String {
    format!(r#"{{"scope":"default","table":"{table}","key":"{key}"}}"#)
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;

// --- the transport ------------------------------------------------------------------------------

#[test]
fn initialize_list_and_call_round_trip_as_an_account() {
    let f = start("roundtrip", |_| {});
    let alice = f.token("alice");

    let r = call(f.port, Some(&alice), INITIALIZE);
    assert_eq!(r.status, 200, "{}", r.head);
    assert_eq!(
        r.header("content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(r.header("connection").as_deref(), Some("close"));
    assert!(
        r.body.contains("\"protocolVersion\":\"2025-11-25\""),
        "{}",
        r.body
    );

    let r = call(f.port, Some(&alice), &list(2));
    assert_eq!(r.status, 200);
    for name in ["lattice_read", "lattice_write", "lattice_subscribe"] {
        assert!(r.body.contains(name), "{}", r.body);
    }

    let r = call(
        f.port,
        Some(&alice),
        &tool(3, "lattice_write", &write_args("kv", "k1", "v1")),
    );
    assert_eq!(r.status, 200);
    assert!(r.body.contains(r#"\"applied\":true"#), "{}", r.body);
    assert!(r.body.contains(r#""isError":false"#), "{}", r.body);

    let r = call(
        f.port,
        Some(&alice),
        &tool(4, "lattice_read", &read_args("kv", "k1")),
    );
    assert!(r.body.contains(r#"\"value\":\"v1\""#), "{}", r.body);
}

#[test]
fn two_principals_are_bound_to_their_own_grants() {
    let f = start("two", |_| {});
    let alice = f.token("alice");
    let bob = f.token("bob");
    let r = call(
        f.port,
        Some(&alice),
        &tool(1, "lattice_write", &write_args("kv", "k1", "v1")),
    );
    assert!(r.body.contains(r#"\"applied\":true"#), "{}", r.body);

    // bob reads what alice wrote (his grant is read on kv).
    let r = call(
        f.port,
        Some(&bob),
        &tool(2, "lattice_read", &read_args("kv", "k1")),
    );
    assert!(r.body.contains(r#"\"value\":\"v1\""#), "{}", r.body);

    // bob holds no write grant on any table, so the write tool is not granted to him at all.
    let r = call(
        f.port,
        Some(&bob),
        &tool(3, "lattice_write", &write_args("kv", "k2", "x")),
    );
    assert!(r.body.contains(r#""code":40301"#), "{}", r.body);
    assert!(r.body.contains("tool not granted"), "{}", r.body);

    // carol holds no bit on kv: a row of kv is the same answer as a row of a table that does not
    // exist (denied == missing), byte for byte, with the same request id.
    let carol_denied = call(
        f.port,
        Some(&f.token("carol")),
        &tool(4, "lattice_read", &read_args("kv", "k1")),
    );
    let carol_missing = call(
        f.port,
        Some(&f.token("carol")),
        &tool(4, "lattice_read", &read_args("ghost", "k1")),
    );
    assert!(
        carol_denied.body.contains(r#"\"error\":\"not_found\""#),
        "{}",
        carol_denied.body
    );
    assert_eq!(carol_denied.body, carol_missing.body);
}

#[test]
fn a_principal_named_in_the_body_is_ignored() {
    let f = start("body-principal", |_| {});
    let bob = f.token("bob");
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{{}},"io.lattice.principal":"alice"}},"name":"lattice_write","arguments":{}}}}}"#,
        write_args("kv", "k9", "forged")
    );
    // The write is refused as bob, who holds no write grant. Had the body been believed, alice's
    // grant would have let it through, so this answer is the proof that the body chose nothing.
    let r = call(f.port, Some(&bob), &body);
    assert!(r.body.contains(r#""code":40301"#), "{}", r.body);
    // And bob's real grant is intact: his read of kv still works after the forged call.
    let r = call(
        f.port,
        Some(&bob),
        &tool(6, "lattice_read", &read_args("kv", "k9")),
    );
    assert!(r.body.contains(r#"\"error\":\"not_found\""#), "{}", r.body);
}

#[test]
fn notifications_are_202_with_no_body_and_bad_json_is_a_parse_error_in_a_200() {
    let f = start("notify", |_| {});
    let alice = f.token("alice");
    let r = call(
        f.port,
        Some(&alice),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(r.status, 202, "{}", r.head);
    assert_eq!(r.body, "");
    assert_eq!(r.header("content-length").as_deref(), Some("0"));

    let r = call(f.port, Some(&alice), "{");
    assert_eq!(r.status, 200);
    assert!(r.body.contains(r#""code":-32700"#), "{}", r.body);

    let r = call(
        f.port,
        Some(&alice),
        r#"{"jsonrpc":"2.0","id":8,"method":"frobnicate"}"#,
    );
    assert_eq!(r.status, 200);
    assert!(r.body.contains(r#""code":-32601"#), "{}", r.body);
}

/// A standard HTTP MCP client (Claude Code, Cursor): `initialize`, `notifications/initialized`,
/// `tools/list` and `tools/call`, with no `_meta` and no session id. Each request is its own message;
/// the calls succeed with no state carried between them.
#[test]
fn a_standard_client_with_no_meta_and_no_session_can_call_tools() {
    let f = start("bare-client", |_| {});

    // alice may write kv: initialize, the notification, the list, and a write.
    let alice = f.token("alice");
    let r = call(f.port, Some(&alice), INITIALIZE);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(
        r.body.contains("\"protocolVersion\":\"2025-11-25\""),
        "{}",
        r.body
    );
    let r = call(
        f.port,
        Some(&alice),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let r = call(
        f.port,
        Some(&alice),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("lattice_write"), "{}", r.body);
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"lattice_write","arguments":{}}}}}"#,
        write_args("kv", "bare", "from-alice")
    );
    let r = call(f.port, Some(&alice), &body);
    assert!(r.body.contains(r#"\"applied\":true"#), "{}", r.body);

    // bob may only read kv: the same sequence, a list without the write tool, and a read of alice's row.
    let bob = f.token("bob");
    let r = call(f.port, Some(&bob), INITIALIZE);
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(
        f.port,
        Some(&bob),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let r = call(
        f.port,
        Some(&bob),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("lattice_read"), "{}", r.body);
    assert!(!r.body.contains("lattice_write"), "{}", r.body);
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"lattice_read","arguments":{}}}}}"#,
        read_args("kv", "bare")
    );
    let r = call(f.port, Some(&bob), &body);
    assert!(r.body.contains(r#"\"value\":\"from-alice\""#), "{}", r.body);
}

/// A `_meta` that is present is still validated: a version that is neither revision is refused, and
/// the 2026-07-28 one is accepted, on a request with no session.
#[test]
fn a_present_meta_is_still_validated_without_a_session() {
    let f = start("bad-meta", |_| {});
    let alice = f.token("alice");
    let bad = r#"{"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#;
    let r = call(f.port, Some(&alice), bad);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains(r#""code":-32022"#), "{}", r.body);
    let good = r#"{"jsonrpc":"2.0","id":6,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#;
    let r = call(f.port, Some(&alice), good);
    assert!(r.body.contains("lattice_read"), "{}", r.body);
}

// --- authentication ------------------------------------------------------------------------------

#[test]
fn missing_and_wrong_credentials_get_the_same_401_bytes() {
    let f = start("uniform", |_| {});
    let missing = call(f.port, None, &list(1));
    let wrong = call(
        f.port,
        Some("lat_0000000000000000_0000000000000000000000000000000000000000"),
        &list(1),
    );
    let other_scheme = exchange(
        f.port,
        &raw(
            "POST",
            "/mcp",
            &[
                format!("Host: localhost:{}", f.port),
                "Content-Type: application/json".to_owned(),
                format!("Content-Length: {}", list(1).len()),
                "Authorization: Basic YWxpY2U6eA==".to_owned(),
            ],
            list(1).as_bytes(),
        ),
    );
    assert_eq!(missing.status, 401, "{}", missing.head);
    assert_eq!(missing.body, "authentication required");
    assert_eq!(
        missing.header("www-authenticate").as_deref(),
        Some("Bearer")
    );
    assert_eq!(wrong.status, 401);
    assert_eq!(other_scheme.status, 401);
    // Byte-identical: a client cannot tell an unknown token from a missing one.
    assert_eq!(missing.head, wrong.head);
    assert_eq!(missing.body, wrong.body);
    assert_eq!(missing.head, other_scheme.head);
}

#[test]
fn a_token_in_the_url_is_refused_and_never_echoed() {
    let f = start("url-token", |_| {});
    let alice = f.token("alice");
    let body = list(1);
    let bytes = raw(
        "POST",
        &format!("/mcp?token={alice}"),
        &headers(f.port, None, body.len()),
        body.as_bytes(),
    );
    let r = exchange(f.port, &bytes);
    assert_eq!(r.status, 400, "{}", r.head);
    assert!(!r.head.contains(&alice) && !r.body.contains(&alice));
}

#[test]
fn repeated_failures_rate_limit_the_endpoint_for_every_token() {
    let f = start("ratelimit", |g| g.failure_limit = 3);
    for _ in 0..3 {
        assert_eq!(call(f.port, None, &list(1)).status, 401);
    }
    // The address is blocked: even the right token is not looked at now.
    let r = call(f.port, Some(&f.token("alice")), &list(1));
    assert_eq!(r.status, 429, "{}", r.head);
}

#[test]
fn a_revoked_token_stops_working_within_the_reload_bound() {
    let (f, file) = start_file("revoke", |_| {});
    let alice = f.token("alice");
    assert_eq!(call(f.port, Some(&alice), &list(1)).status, 200);
    let mut store = load_credential_store(&file).unwrap();
    let id = TokenId::parse(&alice[4..20]).unwrap();
    assert!(store.revoke(id));
    write_credential_store(&file, &store).unwrap();
    // The credential poll is 50 ms; five seconds is the ADR 0031 bound.
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let r = call(f.port, Some(&alice), &list(1));
        if r.status == 401 {
            break;
        }
        assert!(
            Instant::now() < until,
            "still {} after the reload bound",
            r.status
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let bob = f.token("bob");
    assert_eq!(
        call(f.port, Some(&bob), &list(1)).status,
        200,
        "bob is unaffected"
    );
}

#[test]
fn a_gateway_without_a_credential_store_serves_no_mcp_request() {
    let dir = std::env::temp_dir().join(format!("latticed-mcp-http-dev-{}", std::process::id()));
    private_dir(&dir);
    let (store, tokens) = tokens_and_store();
    seed(&dir, &store, &tokens);
    let mut gateway = GatewayConfig {
        auth: GatewayAuth::LocalDevelopment,
        ..GatewayConfig::default()
    };
    fast(&mut gateway);
    gateway.failure_limit = 2;
    let server = boot(&dir, gateway);
    let port = server.gateway_addr().unwrap().port();
    // No token, and no token verifies on this gateway: a valid account's token is refused too.
    // Refusals here are not counted against the address (as on the WebSocket upgrade of this gateway),
    // so they never turn into 429 however many arrive.
    for _ in 0..5 {
        let r = call(port, None, &list(1));
        assert_eq!(r.status, 401, "{}", r.head);
    }
    let r = call(port, None, &list(1));
    assert_eq!(r.status, 401, "{}", r.head);
    let r = call(port, Some(&tokens["admin"]), &list(1));
    assert_eq!(r.status, 401, "{}", r.head);
    let _ = server.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

// --- browser and rebinding defences --------------------------------------------------------------

#[test]
fn only_this_listener_or_localhost_on_its_port_is_a_host() {
    let f = start("host", |_| {});
    let alice = f.token("alice");
    let body = list(1);
    let with_host = |host: &str| {
        let mut h = headers(f.port, Some(&alice), body.len());
        h[0] = format!("Host: {host}");
        exchange(f.port, &raw("POST", "/mcp", &h, body.as_bytes()))
    };
    assert_eq!(with_host(&format!("127.0.0.1:{}", f.port)).status, 200);
    assert_eq!(with_host(&format!("localhost:{}", f.port)).status, 200);
    assert_eq!(with_host(&format!("LOCALHOST:{}", f.port)).status, 200);
    // A name that resolves to loopback (DNS rebinding) is refused, whatever it resolves to.
    assert_eq!(with_host(&format!("rebind.example:{}", f.port)).status, 403);
    assert_eq!(with_host(&format!("127.0.0.1:{}", f.port + 1)).status, 403);
    assert_eq!(with_host("localhost").status, 403);
    assert_eq!(with_host(&format!("[::1]:{}", f.port)).status, 403);
    // Missing or repeated Host is refused as well.
    let mut missing = headers(f.port, Some(&alice), body.len());
    missing.remove(0);
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &missing, body.as_bytes())).status,
        403
    );
    let mut repeated = headers(f.port, Some(&alice), body.len());
    repeated.insert(1, format!("Host: localhost:{}", f.port));
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &repeated, body.as_bytes())).status,
        403
    );
}

#[test]
fn any_origin_is_refused_and_no_cors_header_is_ever_sent() {
    let f = start("origin", |_| {});
    let alice = f.token("alice");
    let body = list(1);
    let origins = [
        "null".to_owned(),
        "http://localhost:3000".to_owned(),
        format!("http://127.0.0.1:{}", f.port),
    ];
    for origin in origins {
        let mut h = headers(f.port, Some(&alice), body.len());
        h.push(format!("Origin: {origin}"));
        let r = exchange(f.port, &raw("POST", "/mcp", &h, body.as_bytes()));
        assert_eq!(r.status, 403, "origin {origin}: {}", r.head);
        assert!(
            !r.body.contains("tools"),
            "origin {origin} reached the tools"
        );
        assert!(!r.head.to_ascii_lowercase().contains("access-control-"));
    }
    // A preflight is refused too (it is an OPTIONS with an Origin).
    let mut h = headers(f.port, None, 0);
    h.push("Origin: http://localhost:3000".to_owned());
    h.push("Access-Control-Request-Method: POST".to_owned());
    let r = exchange(f.port, &raw("OPTIONS", "/mcp", &h, b""));
    assert!(r.status == 403 || r.status == 405, "{}", r.head);
    assert!(!r.head.to_ascii_lowercase().contains("access-control-"));
    // A request with no Origin, as a non-browser client sends it, is served with no CORS header.
    let r = call(f.port, Some(&alice), &list(2));
    assert_eq!(r.status, 200);
    assert!(!r.head.to_ascii_lowercase().contains("access-control-"));
}

#[test]
fn cross_origin_post_cannot_call_a_tool() {
    let f = start("cross", |_| {});
    // A page served from a second loopback origin posts a tool call with the user's token. The page
    // sends its own Origin, and the call is refused before the tool runs.
    let alice = f.token("alice");
    let body = tool(1, "lattice_write", &write_args("kv", "evil", "x"));
    let mut h = headers(f.port, Some(&alice), body.len());
    h.push("Origin: http://localhost:8000".to_owned());
    let r = exchange(f.port, &raw("POST", "/mcp", &h, body.as_bytes()));
    assert_eq!(r.status, 403);
    // The write did not happen: bob reads the key back as missing.
    let bob = f.token("bob");
    let read = call(
        f.port,
        Some(&bob),
        &tool(2, "lattice_read", &read_args("kv", "evil")),
    );
    assert!(
        read.body.contains(r#"\"error\":\"not_found\""#),
        "{}",
        read.body
    );
}

#[test]
fn only_post_is_a_method_and_only_json_is_a_body() {
    let f = start("method", |_| {});
    let alice = f.token("alice");
    let body = list(1);
    let get = exchange(
        f.port,
        &raw("GET", "/mcp", &headers(f.port, Some(&alice), 0), b""),
    );
    assert_eq!(get.status, 405, "{}", get.head);
    assert_eq!(get.header("allow").as_deref(), Some("POST"));
    let put = exchange(
        f.port,
        &raw(
            "PUT",
            "/mcp",
            &headers(f.port, Some(&alice), body.len()),
            body.as_bytes(),
        ),
    );
    assert_eq!(put.status, 405);

    for content_type in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "text/plain;application/json",
    ] {
        let mut h = headers(f.port, Some(&alice), body.len());
        h[1] = format!("Content-Type: {content_type}");
        let r = exchange(f.port, &raw("POST", "/mcp", &h, body.as_bytes()));
        assert_eq!(r.status, 415, "{content_type}: {}", r.head);
    }
    let mut none = headers(f.port, Some(&alice), body.len());
    none.remove(1);
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &none, body.as_bytes())).status,
        415
    );
    // charset=utf-8 is the one parameter accepted.
    let mut charset = headers(f.port, Some(&alice), body.len());
    charset[1] = "Content-Type: application/json; charset=utf-8".to_owned();
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &charset, body.as_bytes())).status,
        200
    );
}

#[test]
fn lengths_are_declared_checked_first_and_bounded() {
    let f = start("length", |_| {});
    let alice = f.token("alice");
    let body = list(1);

    // No Content-Length, and chunked bodies, are refused.
    let mut none = headers(f.port, Some(&alice), 0);
    none.retain(|h| !h.starts_with("Content-Length"));
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &none, body.as_bytes())).status,
        411
    );
    let mut chunked = headers(f.port, Some(&alice), body.len());
    chunked.push("Transfer-Encoding: chunked".to_owned());
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &chunked, body.as_bytes())).status,
        411
    );
    let mut bad = headers(f.port, Some(&alice), body.len());
    bad[2] = "Content-Length: +5".to_owned();
    assert_eq!(
        exchange(f.port, &raw("POST", "/mcp", &bad, body.as_bytes())).status,
        400
    );

    // Over the 1 MiB cap the answer comes at once from the declared length: no body is sent and the
    // server does not wait for one.
    let started = Instant::now();
    let r = exchange(
        f.port,
        &raw(
            "POST",
            "/mcp",
            &headers(f.port, Some(&alice), 1_048_577),
            b"",
        ),
    );
    assert_eq!(r.status, 413, "{}", r.head);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "waited for a body it refuses"
    );

    // Exactly the cap is read and answered (here a whitespace body, a parse error inside a 200).
    let spaces = vec![b' '; 1_048_576];
    let r = exchange(
        f.port,
        &raw(
            "POST",
            "/mcp",
            &headers(f.port, Some(&alice), spaces.len()),
            &spaces,
        ),
    );
    assert_eq!(r.status, 200, "{}", r.head);
}

#[test]
fn a_body_that_stops_short_is_cut_by_the_handshake_deadline() {
    let f = start("short-body", |g| {
        g.handshake_timeout = Duration::from_millis(300)
    });
    let alice = f.token("alice");
    let started = Instant::now();
    let r = exchange(
        f.port,
        &raw(
            "POST",
            "/mcp",
            &headers(f.port, Some(&alice), 100),
            b"{\"jsonrpc\"",
        ),
    );
    // Cut without an answer: the request was not finished in time.
    assert_eq!(r.status, 0, "{}", r.head);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_workspace_that_is_not_an_mcp_scope_stops_the_start() {
    let dir = std::env::temp_dir().join(format!("latticed-mcp-http-ws-{}", std::process::id()));
    private_dir(&dir);
    let (store, tokens) = tokens_and_store();
    seed(&dir, &store, &tokens);
    let mut gateway = GatewayConfig {
        auth: GatewayAuth::Bearer(store),
        mcp_workspace: "not a scope".to_owned(),
        ..GatewayConfig::default()
    };
    fast(&mut gateway);
    let started = SqlServer::start_with_gateway(
        ServerConfig {
            address: SocketAddr::from(([127, 0, 0, 1], 0)),
            ..ServerConfig::default()
        },
        SqlConfig {
            data_dir: dir.clone(),
            policies: policies(),
            limits: HostLimits::default(),
            allow_format_upgrade: false,
        },
        Some(gateway),
    );
    assert!(started.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- configured Host names (--gateway-allowed-host, issue #774) ------------------------------------

/// A server whose `POST /mcp` also answers to `names`, as `--gateway-allowed-host` values.
fn start_with_hosts(name: &str, names: &[&str]) -> Fixture {
    let hosts: Vec<AllowedHost> = names
        .iter()
        .map(|text| AllowedHost::parse(text).unwrap())
        .collect();
    start(name, move |g| g.allowed_hosts = hosts)
}

/// `POST /mcp` as alice with the given `Host` value.
fn call_with_host(f: &Fixture, host: &str) -> Resp {
    let alice = f.token("alice");
    let body = list(1);
    let mut h = headers(f.port, Some(&alice), body.len());
    h[0] = format!("Host: {host}");
    exchange(f.port, &raw("POST", "/mcp", &h, body.as_bytes()))
}

#[test]
fn a_configured_host_is_answered_with_its_port_in_any_case_and_the_defaults_stay() {
    let f = start_with_hosts(
        "allowed-accept",
        &["Gateway.Example.test", "proxy.example.test:9443"],
    );
    let port = f.port;
    // The bound port is the default port of an entry without one, matched in any case.
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test:{port}")).status,
        200
    );
    assert_eq!(
        call_with_host(&f, &format!("GATEWAY.Example.TEST:{port}")).status,
        200
    );
    // An entry with its own port answers to that port.
    assert_eq!(call_with_host(&f, "proxy.example.test:9443").status, 200);
    // The defaults are unchanged.
    assert_eq!(call_with_host(&f, &format!("127.0.0.1:{port}")).status, 200);
    assert_eq!(call_with_host(&f, &format!("localhost:{port}")).status, 200);
    // Another port, a name that was not configured, and a configured name without its port are refused.
    assert_eq!(
        call_with_host(&f, &format!("proxy.example.test:{port}")).status,
        403
    );
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test:{}", port + 1)).status,
        403
    );
    assert_eq!(
        call_with_host(&f, &format!("other.example.test:{port}")).status,
        403
    );
    assert_eq!(call_with_host(&f, "gateway.example.test").status, 403);
    // A name that only resolves to loopback is still a rebinding attempt, configured or not.
    assert_eq!(
        call_with_host(&f, &format!("rebind.example:{port}")).status,
        403
    );
}

#[test]
fn a_host_with_userinfo_a_path_or_whitespace_is_refused_even_for_a_configured_name() {
    let f = start_with_hosts("allowed-junk", &["gateway.example.test"]);
    let port = f.port;
    for host in [
        format!("gateway.example.test@evil.example:{port}"),
        format!("evil.example@gateway.example.test:{port}"),
        format!("gateway.example.test :{port}"),
        format!("gateway example.test:{port}"),
        format!("gateway.example.test:{port}/mcp"),
        format!("gateway.example.test:{port}?x=1"),
    ] {
        assert_eq!(call_with_host(&f, &host).status, 403, "Host: {host}");
    }
    // A control character in the value is malformed before the Host check runs.
    let r = call_with_host(&f, &format!("gateway.example.test:{port}\u{1}"));
    assert_eq!(r.status, 400, "{}", r.head);
    // The configured name itself, unchanged, is served.
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test:{port}")).status,
        200
    );
}

#[test]
fn a_host_is_compared_as_written_so_a_trailing_dot_or_a_leading_zero_port_is_refused() {
    let f = start_with_hosts("allowed-as-written", &["gateway.example.test"]);
    let port = f.port;
    // The name is compared as the client wrote it: a fully qualified form with a trailing dot, and a port
    // written with a leading zero, are different strings and are refused.
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test.:{port}")).status,
        403
    );
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test:0{port}")).status,
        403
    );
    assert_eq!(
        call_with_host(&f, &format!("gateway.example.test:{port}")).status,
        200
    );
}

#[test]
fn the_default_hosts_still_work_without_any_configured_name() {
    let f = start("allowed-none", |_| {});
    let port = f.port;
    assert_eq!(call_with_host(&f, &format!("127.0.0.1:{port}")).status, 200);
    assert_eq!(call_with_host(&f, &format!("localhost:{port}")).status, 200);
    assert_eq!(
        call_with_host(&f, "gateway.example.test").status,
        403,
        "an unconfigured name is refused"
    );
}
