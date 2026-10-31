//! `lattice watch table/NAME`: prints the live changes of one table from the sync gateway.
//!
//! This part holds what does not need a socket: argument parsing, the loopback-only and token
//! rules, the event line format, and the driver that prints each event before it acknowledges it.
//! The driver is generic over [`Connection`] and [`Output`], so the order rule is tested without a
//! network. The socket transport is not in this part, so a valid invocation stops at its first
//! write with a message that says so.

use lattice_core::feed::{ChangeEvent, ChangeOp, Cursor};
use lattice_sdk::client::{Action, ClientConfig, Frame, SyncClient};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::process::ExitCode;
use zeroize::Zeroizing;

/// The largest token file or standard input read, in bytes.
const MAX_TOKEN_BYTES: u64 = 4096;
/// The name this client gives itself in `Hello`.
const AGENT: &str = "lattice-watch";
/// The prefix of a table's sync scope.
const SCOPE_PREFIX: &str = "table/";
/// Why a valid invocation cannot connect yet.
const NO_TRANSPORT: &str =
    "the socket transport for `lattice watch` is not in this build yet (issue #583)";
const TOKEN_ON_COMMAND_LINE: &str = "a token is not accepted on the command line; write it to a file and pass --token-file FILE (or --token-file - for standard input)";
const CURSOR_HINT: &str =
    "--from takes a cursor of 24 lowercase hex digits, as printed at the start of each line";

const USAGE: &str = "\
usage: lattice watch table/NAME --url ws://HOST:PORT --token-file FILE (--from CURSOR | --from-start)

  --url URL          a ws:// URL on a loopback address (127.0.0.1, localhost or [::1]); no TLS yet
  --token-file FILE  the bearer token, read from FILE, or from standard input with -
  --from CURSOR      resume after CURSOR: 24 lowercase hex digits, as printed at the start of a line
  --from-start       deliver every event the table holds
  -h, --help         print this text

Each event prints one line: CURSOR AUTHOR TABLE KEY OPERATION
The token is never accepted on the command line.
";

/// Where the bearer token comes from.
#[derive(Debug, PartialEq, Eq)]
enum TokenSource {
    /// A file holding the token on one line.
    File(String),
    /// Standard input, written as `-`.
    Stdin,
}

/// A checked `watch` invocation.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    table: String,
    url: String,
    token: TokenSource,
    /// Empty to deliver every event the table holds, otherwise the cursor to resume after.
    from: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    Run(Options),
}

/// The byte stream of one watch: frames out and frames in. `None` from `recv` means the peer closed
/// the stream.
trait Connection {
    fn send(&mut self, frame: &Frame) -> Result<(), String>;
    fn recv(&mut self) -> Result<Option<Frame>, String>;
}

/// Where event lines go. A line is flushed before `line` returns, so an acknowledgement written
/// afterwards can never be ahead of the text it covers.
trait Output {
    fn line(&mut self, text: &str) -> Result<(), String>;
}

fn set_once<T>(slot: &mut Option<T>, flag: &str, new: T) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{flag} is given twice"));
    }
    *slot = Some(new);
    Ok(())
}

fn value<'a>(rest: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<&'a str, String> {
    match rest.next() {
        Some(v) if !v.starts_with("--") => Ok(v.as_str()),
        _ => Err(format!("{flag} needs a value")),
    }
}

/// `table/NAME` with NAME one to 64 of `a-z`, `0-9` or `_`: the names the catalog holds.
fn table_of(scope: &str) -> Result<String, String> {
    let name = scope
        .strip_prefix(SCOPE_PREFIX)
        .ok_or_else(|| "the table is named table/NAME".to_owned())?;
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "`{name}` is not a table name: 1 to 64 of a-z, 0-9 or _"
        ))
    }
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

/// Exactly 24 lowercase hex digits that encode a valid feed cursor.
fn parse_cursor(text: &str) -> Result<Vec<u8>, String> {
    let digits = text.as_bytes();
    if digits.len() != 2 * Cursor::ENCODED_LEN {
        return Err(CURSOR_HINT.into());
    }
    let (pairs, _) = digits.as_chunks::<2>();
    let mut bytes = Vec::with_capacity(Cursor::ENCODED_LEN);
    for &[hi, lo] in pairs {
        match (nibble(hi), nibble(lo)) {
            (Some(h), Some(l)) => bytes.push((h << 4) | l),
            _ => return Err(CURSOR_HINT.into()),
        }
    }
    if Cursor::from_bytes(&bytes).is_err() {
        return Err(CURSOR_HINT.into());
    }
    Ok(bytes)
}

/// `host` and `port` of an authority, where a bracketed IPv6 host keeps its brackets.
fn split_host_port(authority: &str) -> Option<(&str, &str)> {
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let port = authority.get(end + 1..)?.strip_prefix(':')?;
        Some((authority.get(..=end)?, port))
    } else {
        authority.rsplit_once(':')
    }
}

/// Whether `host` names a loopback address. `localhost` is accepted by name here. A future socket
/// transport must connect to the loopback address this check accepted (127.0.0.1 or ::1), and must
/// not resolve the name again: a second lookup could return a remote address.
fn is_loopback(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return v6.parse::<Ipv6Addr>().is_ok_and(|a| a.is_loopback());
    }
    host.parse::<Ipv4Addr>().is_ok_and(|a| a.is_loopback())
}

/// Only a `ws://` URL on a loopback address with an explicit port. There is no TLS on the client
/// yet, so a remote gateway is refused rather than reached in clear text.
fn check_url(url: &str) -> Result<(), String> {
    if url.starts_with("wss://") {
        return Err("wss:// (TLS) is not supported yet; use ws:// on a loopback address".into());
    }
    let Some(rest) = url.strip_prefix("ws://") else {
        return Err("the URL must start with ws:// (for example ws://127.0.0.1:7000)".into());
    };
    if rest.contains(['?', '#', '@']) {
        return Err("the URL must not carry a query, a fragment or user information".into());
    }
    let authority = rest.split('/').next().unwrap_or(rest);
    let Some((host, port)) = split_host_port(authority) else {
        return Err("the URL needs an explicit port, for example ws://127.0.0.1:7000".into());
    };
    let port: u16 = port
        .parse()
        .map_err(|_| format!("`{port}` is not a port number"))?;
    if port == 0 {
        return Err("the port must not be 0".into());
    }
    if !is_loopback(host) {
        return Err(format!(
            "refusing `{host}`: watch connects only to a loopback address (127.0.0.1, localhost or [::1]) because it has no TLS yet"
        ));
    }
    Ok(())
}

/// Reads the whole argument list. Pure: nothing is opened or read here.
fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut table = None;
    let mut url = None;
    let mut token = None;
    let mut from = None;
    let mut from_start = false;
    let mut help = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "--url" => set_once(&mut url, "--url", value(&mut rest, "--url")?.to_owned())?,
            "--token-file" => set_once(
                &mut token,
                "--token-file",
                value(&mut rest, "--token-file")?.to_owned(),
            )?,
            "--from" => set_once(
                &mut from,
                "--from",
                parse_cursor(value(&mut rest, "--from")?)?,
            )?,
            "--from-start" => {
                if from_start {
                    return Err("--from-start is given twice".into());
                }
                from_start = true;
            }
            "--token" => return Err(TOKEN_ON_COMMAND_LINE.into()),
            other if other.starts_with("--token=") => return Err(TOKEN_ON_COMMAND_LINE.into()),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if table.is_some() {
                    return Err("only one table may be named".into());
                }
                table = Some(table_of(other)?);
            }
        }
    }
    if help {
        return Ok(Parsed::Help);
    }
    let table = table.ok_or("missing the table: watch table/NAME")?;
    let url = url.ok_or("missing --url ws://HOST:PORT")?;
    check_url(&url)?;
    let token = token.ok_or("missing --token-file FILE")?;
    let from = match (from, from_start) {
        (Some(cursor), false) => cursor,
        (None, true) => Vec::new(),
        (None, false) => return Err("give --from CURSOR or --from-start".into()),
        (Some(_), true) => return Err("--from and --from-start cannot both be given".into()),
    };
    let token = if token == "-" {
        TokenSource::Stdin
    } else {
        TokenSource::File(token)
    };
    Ok(Parsed::Run(Options {
        table,
        url,
        token,
        from,
    }))
}

/// The token: one line, printable ASCII without spaces, at most `MAX_TOKEN_BYTES`, with the line
/// ending not part of it. A token file gets the same checks as the other credential files
/// (`latticed::files::read_private`): a regular file, not a symlink, with no group or other access,
/// in a directory that group and others cannot write to. Errors name the path and the rule, never
/// the contents.
fn read_token(source: &TokenSource, stdin: &mut dyn Read) -> Result<Zeroizing<String>, String> {
    let limit = MAX_TOKEN_BYTES as usize;
    let raw: Zeroizing<Vec<u8>> = match source {
        TokenSource::Stdin => {
            let mut bytes: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
            stdin
                .take(MAX_TOKEN_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read the token: {e}"))?;
            bytes
        }
        TokenSource::File(path) => latticed::files::read_private(Path::new(path), limit)
            .map_err(|e| format!("{path}: {e}"))?,
    };
    if raw.len() > limit {
        return Err(format!("the token is larger than {MAX_TOKEN_BYTES} bytes"));
    }
    let not_a_token = "the token must be one line of printable ASCII with no spaces";
    let text = std::str::from_utf8(&raw).map_err(|_| not_a_token.to_owned())?;
    let body = text
        .strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(text);
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(not_a_token.into());
    }
    Ok(Zeroizing::new(body.to_owned()))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            s.push_str(&format!("{b:02x}"));
            s
        })
}

/// A key as a quoted, escaped string, so a key cannot forge a line break or a terminal escape. A key
/// that is not UTF-8 is shown as `x"HEX"`.
fn key_text(key: &[u8]) -> String {
    match std::str::from_utf8(key) {
        Ok(text) => format!("{text:?}"),
        Err(_) => format!("x\"{}\"", hex(key)),
    }
}

/// `CURSOR AUTHOR TABLE KEY OPERATION`. The cursor is the form `--from` takes.
fn format_event(table: &str, event: &ChangeEvent) -> String {
    let operation = match event.change.op {
        ChangeOp::Delete => "delete",
        ChangeOp::Upsert(_) => "upsert",
    };
    format!(
        "{} {:032x} {table} {} {operation}",
        hex(&event.cursor.to_bytes()),
        event.author,
        key_text(&event.change.key),
    )
}

/// Decodes one delivered event and checks that it is the event its frame names.
fn decode_delivery(cursor: &[u8], data: &[u8]) -> Result<ChangeEvent, String> {
    let event = ChangeEvent::decode(data).map_err(|e| format!("undecodable event: {e}"))?;
    if event.cursor.to_bytes().as_slice() != cursor {
        return Err("an event does not carry the cursor of its delivery".into());
    }
    Ok(event)
}

/// One sync client over one connection. Every action runs before the next one starts, so an
/// acknowledgement is only written after its event's line has been printed.
struct Pump<'a, C: Connection, O: Output> {
    client: &'a mut SyncClient,
    table: &'a str,
    from: &'a [u8],
    conn: &'a mut C,
    out: &'a mut O,
}

impl<C: Connection, O: Output> Pump<'_, C, O> {
    fn perform(&mut self, actions: Vec<Action>) -> Result<(), String> {
        for action in actions {
            match action {
                Action::Send(frame) => self.conn.send(&frame)?,
                Action::Ready { .. } => {
                    let scope = format!("{SCOPE_PREFIX}{}", self.table);
                    let (_, subscribe) = self
                        .client
                        .subscribe(&scope, self.from)
                        .map_err(|e| e.to_string())?;
                    self.perform(subscribe)?;
                }
                Action::Deliver { sub, cursor, data } => {
                    let event = decode_delivery(&cursor, &data)?;
                    self.out.line(&format_event(self.table, &event))?;
                    let acks = self.client.ack(sub, &cursor).map_err(|e| e.to_string())?;
                    self.perform(acks)?;
                }
                Action::SubscriptionFailed { code, message, .. } => {
                    return Err(format!(
                        "the server refused the subscription ({code:?}): {message:?}"
                    ));
                }
                Action::Close(reason) => {
                    return Err(format!("the connection closed: {reason:?}"));
                }
            }
        }
        Ok(())
    }
}

/// Runs one watch over `conn`. It returns only with an error: a watch ends when it is interrupted
/// or when the connection fails. The reconnect loop is not in this part.
fn run<C: Connection, O: Output>(
    table: &str,
    from: &[u8],
    conn: &mut C,
    out: &mut O,
) -> Result<(), String> {
    let mut client = SyncClient::new(ClientConfig::new(AGENT)).map_err(|e| e.to_string())?;
    let mut pump = Pump {
        client: &mut client,
        table,
        from,
        conn,
        out,
    };
    let mut now: u64 = 0;
    let hello = pump.client.connect(now).map_err(|e| e.to_string())?;
    pump.perform(hello)?;
    loop {
        let Some(frame) = pump.conn.recv()? else {
            return Err("the server closed the connection".into());
        };
        now = now.saturating_add(1);
        let actions = pump.client.handle(frame, now).map_err(|e| e.to_string())?;
        pump.perform(actions)?;
    }
}

/// Event lines on standard output, flushed one by one.
struct Printer;

impl Output for Printer {
    fn line(&mut self, text: &str) -> Result<(), String> {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{text}").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())
    }
}

/// The connection this part can offer: none. Its first write fails with the reason.
struct Unconnected {
    url: String,
}

impl Connection for Unconnected {
    fn send(&mut self, _frame: &Frame) -> Result<(), String> {
        Err(self.refusal())
    }

    fn recv(&mut self) -> Result<Option<Frame>, String> {
        Err(self.refusal())
    }
}

impl Unconnected {
    fn refusal(&self) -> String {
        format!("cannot open {}: {NO_TRANSPORT}", self.url)
    }
}

fn start(opts: Options) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    // Read and checked now, so a later transport takes the token without a second read.
    let _token = read_token(&opts.token, &mut lock)?;
    let mut conn = Unconnected { url: opts.url };
    run(&opts.table, &opts.from, &mut conn, &mut Printer)
}

/// `lattice watch ...`: exit 0 for help, 2 for a bad invocation, 1 when the watch cannot run.
pub(crate) fn command(args: &[String]) -> ExitCode {
    match parse(args) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(opts)) => match start(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("lattice watch: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("lattice watch: {e}");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use lattice_sdk::client::{Event, SUPPORTED_MAX_VERSION, SUPPORTED_MIN_VERSION};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// Hex with spaces allowed, as written in `formats/change-feed-v1.md`.
    fn hex_bytes(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        let (pairs, _) = digits.as_chunks::<2>();
        pairs
            .iter()
            .map(|&[hi, lo]| (nibble(hi).unwrap() << 4) | nibble(lo).unwrap())
            .collect()
    }

    /// The upsert of key "k" to "vv" at index `index` of a change set of three events, LSN 258.
    /// The bytes are the worked example of `formats/change-feed-v1.md`, with the index changed.
    fn upsert_event(index: &str) -> String {
        format!(
            "01 0000000000000102 {index} 03000000 {} 0500000000000000 06000000 07000000 {} 00 02000000 0100 6b 01 02000000 7676",
            "a0".repeat(16),
            "00000000000000000000000000000009"
        )
    }

    /// The delete with a turn from `formats/change-feed-v1.md`: LSN 1, index 0, empty key.
    const DELETE_EVENT: &str = "01 0000000000000001 00000000 01000000 01010101010101010101010101010101 00000000000000000000000000000000 00000000000000000000000000000000 01 1100000000000000 2200000000000000 00000000 0000 00";

    const CURSOR_1: &str = "000000000000010200000001";
    const CURSOR_2: &str = "000000000000010200000002";
    const LINE_1: &str =
        "000000000000010200000001 00000000000000000000000000000009 notes \"k\" upsert";
    const LINE_2: &str =
        "000000000000010200000002 00000000000000000000000000000009 notes \"k\" upsert";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[derive(Debug, PartialEq)]
    enum Step {
        Sent(Frame),
        Printed(String),
    }

    type Log = Rc<RefCell<Vec<Step>>>;

    struct Script {
        incoming: VecDeque<Frame>,
        log: Log,
    }

    impl Connection for Script {
        fn send(&mut self, frame: &Frame) -> Result<(), String> {
            self.log.borrow_mut().push(Step::Sent(frame.clone()));
            Ok(())
        }

        fn recv(&mut self) -> Result<Option<Frame>, String> {
            Ok(self.incoming.pop_front())
        }
    }

    struct Recorder {
        log: Log,
    }

    impl Output for Recorder {
        fn line(&mut self, text: &str) -> Result<(), String> {
            self.log.borrow_mut().push(Step::Printed(text.to_owned()));
            Ok(())
        }
    }

    fn hello() -> Frame {
        Frame::Hello {
            min_version: SUPPORTED_MIN_VERSION,
            max_version: SUPPORTED_MAX_VERSION,
            agent: AGENT.into(),
        }
    }

    fn welcome() -> Frame {
        Frame::Welcome {
            version: SUPPORTED_MAX_VERSION,
            max_payload: 1 << 20,
            server: "test".into(),
        }
    }

    /// Runs one watch over the frames in `incoming`. Returns every step in order and the error
    /// that ended the run.
    fn watch_over(incoming: Vec<Frame>, from: &[u8]) -> (Vec<Step>, String) {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut conn = Script {
            incoming: incoming.into(),
            log: Rc::clone(&log),
        };
        let mut out = Recorder {
            log: Rc::clone(&log),
        };
        let err = run("notes", from, &mut conn, &mut out).unwrap_err();
        let steps = std::mem::take(&mut *log.borrow_mut());
        (steps, err)
    }

    #[test]
    fn watch_never_acks_before_the_line_is_printed() {
        let c1 = hex_bytes(CURSOR_1);
        let c2 = hex_bytes(CURSOR_2);
        let incoming = vec![
            welcome(),
            Frame::Events {
                sub: 1,
                events: vec![Event {
                    cursor: c1.clone(),
                    data: hex_bytes(&upsert_event("00000001")),
                }],
            },
            Frame::Events {
                sub: 1,
                events: vec![Event {
                    cursor: c2.clone(),
                    data: hex_bytes(&upsert_event("00000002")),
                }],
            },
        ];
        let (steps, err) = watch_over(incoming, &[]);
        assert!(err.contains("closed"), "{err}");
        // An acknowledgement sent before its line would appear before the Printed step it covers
        // and fail this exact sequence.
        assert_eq!(
            steps,
            vec![
                Step::Sent(hello()),
                Step::Sent(Frame::Subscribe {
                    sub: 1,
                    scope: "table/notes".into(),
                    cursor: Vec::new(),
                }),
                Step::Printed(LINE_1.into()),
                Step::Sent(Frame::Ack { sub: 1, cursor: c1 }),
                Step::Printed(LINE_2.into()),
                Step::Sent(Frame::Ack { sub: 1, cursor: c2 }),
            ]
        );
    }

    #[test]
    fn a_delivery_whose_cursor_differs_from_its_event_is_refused_before_anything_is_printed() {
        let incoming = vec![
            welcome(),
            Frame::Events {
                sub: 1,
                events: vec![Event {
                    cursor: hex_bytes(CURSOR_2),
                    data: hex_bytes(&upsert_event("00000001")),
                }],
            },
        ];
        let (steps, err) = watch_over(incoming, &[]);
        assert!(err.contains("does not carry the cursor"), "{err}");
        assert_eq!(
            steps,
            vec![
                Step::Sent(hello()),
                Step::Sent(Frame::Subscribe {
                    sub: 1,
                    scope: "table/notes".into(),
                    cursor: Vec::new(),
                }),
            ]
        );
    }

    #[test]
    fn an_undecodable_event_ends_the_watch_without_a_line_or_an_ack() {
        let incoming = vec![
            welcome(),
            Frame::Events {
                sub: 1,
                events: vec![Event {
                    cursor: hex_bytes(CURSOR_1),
                    data: vec![9, 9, 9],
                }],
            },
        ];
        let (steps, err) = watch_over(incoming, &[]);
        assert!(err.contains("undecodable event"), "{err}");
        assert!(
            steps
                .iter()
                .all(|s| !matches!(s, Step::Printed(_) | Step::Sent(Frame::Ack { .. })))
        );
    }

    #[test]
    fn an_upsert_line_names_cursor_author_table_key_and_operation() {
        let event = ChangeEvent::decode(&hex_bytes(&upsert_event("00000001"))).unwrap();
        assert_eq!(format_event("notes", &event), LINE_1);
    }

    #[test]
    fn a_delete_line_shows_the_empty_key_as_empty_quotes() {
        let event = ChangeEvent::decode(&hex_bytes(DELETE_EVENT)).unwrap();
        assert_eq!(
            format_event("notes", &event),
            "000000000000000100000000 00000000000000000000000000000000 notes \"\" delete"
        );
    }

    #[test]
    fn keys_are_quoted_and_escaped_so_a_key_cannot_forge_a_line() {
        assert_eq!(key_text(b"a b"), "\"a b\"");
        assert_eq!(key_text(b"x\x1b[2J"), "\"x\\u{1b}[2J\"");
        assert_eq!(key_text(b"line\nbreak"), "\"line\\nbreak\"");
        assert_eq!(key_text(&[0xff]), "x\"ff\"");
        assert_eq!(key_text(b""), "\"\"");
    }

    #[test]
    fn from_takes_exactly_24_lowercase_hex_digits_that_name_a_valid_cursor() {
        assert_eq!(parse_cursor(CURSOR_1).unwrap(), hex_bytes(CURSOR_1));
        assert!(parse_cursor("000000000000010200000001A").is_err());
        assert!(parse_cursor("000000000000010200000001").is_ok());
        assert!(parse_cursor("00000000000001020000000A").is_err());
        assert!(parse_cursor("00000000000001020000000").is_err());
        assert!(parse_cursor("0000000000000102000000011").is_err());
        assert!(parse_cursor("0000000000000102000000g1").is_err());
        // LSN 0 with a non-zero index is not a cursor.
        assert!(parse_cursor("000000000000000000000001").is_err());
    }

    #[test]
    fn only_loopback_ws_urls_with_a_port_are_accepted() {
        for ok in [
            "ws://127.0.0.1:7000",
            "ws://127.0.0.2:9",
            "ws://localhost:7000/",
            "ws://[::1]:7000",
            "ws://127.0.0.1:7000/sync",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "wss://127.0.0.1:7000",
            "http://127.0.0.1:7000",
            "ws://192.0.2.10:7000",
            "ws://0.0.0.0:7000",
            "ws://example.test:7000",
            "ws://localhost.example:7000",
            "ws://127.0.0.1",
            "ws://127.0.0.1:0",
            "ws://127.0.0.1:70000",
            "ws://user@127.0.0.1:7000",
            "ws://127.0.0.1:7000/?token=x",
            "ws://127.0.0.1@evil.com",
            "ws://127.0.0.1@evil.com:5433",
            "WS://127.0.0.1:5433/sync",
            "ws://127.0.0.1.evil.com:5433",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
        let tls = check_url("wss://127.0.0.1:7000").unwrap_err();
        assert!(tls.contains("TLS"), "{tls}");
        let remote = check_url("ws://192.0.2.10:7000").unwrap_err();
        assert!(remote.contains("loopback"), "{remote}");
    }

    #[test]
    fn the_token_is_one_trimmed_line_of_printable_ascii() {
        let token = |bytes: &[u8]| {
            let mut input = bytes;
            read_token(&TokenSource::Stdin, &mut input).map(|t| t.as_str().to_owned())
        };
        assert_eq!(token(b"abc\n").unwrap(), "abc");
        assert_eq!(token(b"abc\r\n").unwrap(), "abc");
        assert_eq!(token(b"abc").unwrap(), "abc");
        assert!(token(b"").is_err());
        assert!(token(b"\n").is_err());
        assert!(token(b"a b\n").is_err());
        assert!(token(b"a\tb\n").is_err());
        assert!(token(b"\xff\n").is_err());
        let mut long = vec![b'a'; 4096];
        assert!(token(&long).is_ok());
        long.push(b'a');
        assert!(token(&long).is_err());
    }

    #[test]
    fn a_valid_invocation_reads_its_arguments_and_names_the_stdin_token() {
        let parsed = parse(&args(&[
            "table/notes",
            "--url",
            "ws://127.0.0.1:7000",
            "--token-file",
            "-",
            "--from-start",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Parsed::Run(Options {
                table: "notes".into(),
                url: "ws://127.0.0.1:7000".into(),
                token: TokenSource::Stdin,
                from: Vec::new(),
            })
        );
        let parsed = parse(&args(&[
            "table/notes",
            "--token-file",
            "/tmp/token",
            "--from",
            CURSOR_1,
            "--url",
            "ws://localhost:7000",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Parsed::Run(Options {
                table: "notes".into(),
                url: "ws://localhost:7000".into(),
                token: TokenSource::File("/tmp/token".into()),
                from: hex_bytes(CURSOR_1),
            })
        );
    }

    #[test]
    fn a_token_on_the_command_line_is_refused_without_echoing_it() {
        for flag in [vec!["--token", "s3cret"], vec!["--token=s3cret"]] {
            let mut list = vec![
                "table/notes",
                "--url",
                "ws://127.0.0.1:7000",
                "--from-start",
            ];
            list.extend(flag);
            let err = parse(&args(&list)).unwrap_err();
            assert!(err.contains("command line"), "{err}");
            assert!(!err.contains("s3cret"), "{err}");
        }
    }

    #[test]
    fn the_start_position_is_exactly_one_of_from_and_from_start() {
        let base = [
            "table/notes",
            "--url",
            "ws://127.0.0.1:7000",
            "--token-file",
            "-",
        ];
        let neither = parse(&args(&base)).unwrap_err();
        assert!(neither.contains("--from-start"), "{neither}");
        let mut both = base.to_vec();
        both.extend(["--from-start", "--from", CURSOR_1]);
        let err = parse(&args(&both)).unwrap_err();
        assert!(err.contains("cannot both"), "{err}");
    }

    #[test]
    fn unknown_repeated_and_missing_parts_are_refused() {
        let good = [
            "table/notes",
            "--url",
            "ws://127.0.0.1:7000",
            "--token-file",
            "-",
            "--from-start",
        ];
        assert!(parse(&args(&good)).is_ok());
        let mut unknown = good.to_vec();
        unknown.push("--frobnicate");
        assert!(
            parse(&args(&unknown))
                .unwrap_err()
                .contains("unknown option")
        );
        let mut twice = good.to_vec();
        twice.extend(["--url", "ws://127.0.0.1:7001"]);
        assert!(parse(&args(&twice)).unwrap_err().contains("given twice"));
        let mut two_tables = good.to_vec();
        two_tables.push("table/other");
        assert!(
            parse(&args(&two_tables))
                .unwrap_err()
                .contains("only one table")
        );
        let no_prefix = [
            "notes",
            "--url",
            "ws://127.0.0.1:7000",
            "--token-file",
            "-",
            "--from-start",
        ];
        assert!(parse(&args(&no_prefix)).is_err());
        let no_url = ["table/notes", "--token-file", "-", "--from-start"];
        assert!(parse(&args(&no_url)).unwrap_err().contains("--url"));
        let no_token = [
            "table/notes",
            "--url",
            "ws://127.0.0.1:7000",
            "--from-start",
        ];
        assert!(
            parse(&args(&no_token))
                .unwrap_err()
                .contains("--token-file")
        );
        let no_value = ["table/notes", "--url", "--from-start"];
        assert!(
            parse(&args(&no_value))
                .unwrap_err()
                .contains("needs a value")
        );
    }

    #[test]
    fn table_names_follow_the_catalog_rule() {
        assert_eq!(table_of("table/notes_2").unwrap(), "notes_2");
        for bad in ["table/", "table/Notes", "table/a-b", "notes", "table/a.b"] {
            assert!(table_of(bad).is_err(), "{bad}");
        }
        assert!(table_of(&format!("table/{}", "a".repeat(64))).is_ok());
        assert!(table_of(&format!("table/{}", "a".repeat(65))).is_err());
    }

    #[test]
    fn help_is_asked_by_either_spelling_and_needs_no_other_argument() {
        assert_eq!(parse(&args(&["--help"])).unwrap(), Parsed::Help);
        assert_eq!(parse(&args(&["table/notes", "-h"])).unwrap(), Parsed::Help);
    }
}
