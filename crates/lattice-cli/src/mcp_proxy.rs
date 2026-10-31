//! `lattice mcp`: a stdio proxy to the gateway's MCP endpoint, `POST /mcp` (issue #775).
//!
//! Most agent clients speak MCP over stdio, and the gateway serves it only as `POST /mcp` with a
//! bearer token. This command reads one JSON-RPC message per line from standard input, sends each
//! as one `POST /mcp` request, and writes each reply as one line on standard output. Diagnostics go
//! to standard error, and the token is never printed.
//!
//! The rules, all checked before anything is sent:
//! - The URL is `http://` on a loopback address with a port, and its path is `/mcp`. The CLI has no
//!   TLS client, so `https://` and a remote address are refused. Connections go to the loopback
//!   address the check accepted, never to a second lookup of the name.
//! - The token comes from an owner-only file (`latticed::files::read_private`), or from the first
//!   line of standard input with `--token-file -`. A token on the command line is refused.
//! - A line longer than the gateway's 1 MiB body limit is dropped as it is read and answered with a
//!   JSON-RPC error. The proxy never holds more than one line in memory.
//! - A reply is passed through byte for byte and is not parsed. A reply that is not HTTP 200 becomes
//!   a JSON-RPC error that does not repeat the server's text, because that text is not JSON.
//!
//! Out of scope: server push (SSE and GET streams), sessions and batching.

use serde_json::Value;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;
use zeroize::Zeroizing;

/// The largest message the gateway accepts as a request body (1 MiB). A longer line is refused.
const MAX_LINE_BYTES: usize = 1 << 20;
/// The largest token file or first line of standard input, in bytes.
const MAX_TOKEN_BYTES: usize = 4096;
/// The largest reply read, head and body together.
const MAX_REPLY_BYTES: u64 = 4 << 20;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Longer than the 30 second wait of a long `lattice_subscribe`, so a waiting reply is not cut off.
const IO_TIMEOUT: Duration = Duration::from_secs(60);
const INVALID_REQUEST: i64 = -32600;
const INTERNAL_ERROR: i64 = -32603;
const TOKEN_ON_COMMAND_LINE: &str = "a token is not accepted on the command line; write it to a file and pass --token-file FILE (or --token-file - to read the first line of standard input)";
const TLS_NOT_SUPPORTED: &str =
    "https:// (TLS) is not supported by lattice mcp yet; use http:// on a loopback address";

const USAGE: &str = "\
usage: lattice mcp --url http://127.0.0.1:PORT/mcp --token-file FILE

Reads one JSON-RPC message per line on standard input, sends each to the gateway's MCP endpoint
and writes each reply as one line on standard output. Diagnostics go to standard error.

  --url URL          the endpoint: http:// on a loopback address (127.0.0.1, localhost or [::1])
                     with a port, and the path /mcp; TLS is not available in this client yet.
                     localhost connects to 127.0.0.1 only: a gateway bound to [::1] needs
                     http://[::1]:PORT/mcp
  --token-file FILE  the bearer token, read from FILE (owner-only, mode 0600), or from the first
                     line of standard input with -; a token on the command line is refused
";

/// Where the bearer token comes from.
enum TokenSource {
    File(String),
    Stdin,
}

/// A loopback socket address to connect to, and the `Host` value to send: the authority exactly as
/// the URL wrote it, which the gateway compares with its own bound address.
struct Endpoint {
    addr: SocketAddr,
    host: String,
    /// Whether the URL named `localhost`, which connects to 127.0.0.1 only.
    localhost: bool,
}

struct Options {
    endpoint: Endpoint,
    token: TokenSource,
}

enum Parsed {
    Help,
    Run(Options),
}

/// One unit read from a line-framed input.
#[derive(Debug, PartialEq, Eq)]
enum Framed {
    /// A line without its `\n`. The last line of the input may lack the `\n`.
    Line(Vec<u8>),
    /// A line over the limit. Its bytes were dropped as they were read, up to its `\n`.
    TooLong,
    /// The input ended before any byte of a new line.
    End,
}

/// What the gateway answered for one message.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The reply body: one JSON-RPC message, not yet framed.
    Reply(Vec<u8>),
    /// The gateway accepted a notification and sent nothing back.
    Accepted,
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

fn not_loopback(host: &str) -> String {
    format!(
        "refusing `{host}`: lattice mcp connects only to a loopback address (127.0.0.1, localhost or [::1]) because the CLI has no TLS yet"
    )
}

/// `host` and `port` of an authority. A bracketed IPv6 host is returned without its brackets.
fn split_authority(authority: &str) -> Option<(&str, &str)> {
    if let Some(inner) = authority.strip_prefix('[') {
        let (host, rest) = inner.split_once(']')?;
        Some((host, rest.strip_prefix(':')?))
    } else {
        authority.rsplit_once(':')
    }
}

/// Only `http://` on a loopback address with an explicit port and the path `/mcp`. The host is
/// resolved here by parsing, never by a lookup, so the address checked is the one connected to.
fn check_url(url: &str) -> Result<Endpoint, String> {
    if url.starts_with("https://") {
        return Err(TLS_NOT_SUPPORTED.into());
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(
            "the URL must start with http:// (for example http://127.0.0.1:7000/mcp)".into(),
        );
    };
    if rest.contains(['?', '#', '@']) {
        return Err("the URL must not carry a query, a fragment or user information".into());
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !path.is_empty() && path != "mcp" {
        return Err("the path must be /mcp, the gateway's MCP endpoint".into());
    }
    let Some((host, port)) = split_authority(authority) else {
        return Err("the URL needs an explicit port, for example http://127.0.0.1:7000/mcp".into());
    };
    let port: u16 = port
        .parse()
        .map_err(|_| format!("`{port}` is not a port number"))?;
    if port == 0 {
        return Err("the port must not be 0".into());
    }
    let ip = if host == "localhost" {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        host.parse::<IpAddr>().map_err(|_| not_loopback(host))?
    };
    if !ip.is_loopback() {
        return Err(not_loopback(host));
    }
    Ok(Endpoint {
        addr: SocketAddr::new(ip, port),
        host: authority.to_owned(),
        localhost: host == "localhost",
    })
}

/// Reads the whole argument list. Pure: nothing is opened or read here.
fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut url = None;
    let mut token = None;
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
            "--token" => return Err(TOKEN_ON_COMMAND_LINE.into()),
            other if other.starts_with("--token=") => return Err(TOKEN_ON_COMMAND_LINE.into()),
            // Not echoed: an unexpected argument may be a secret typed in the wrong place.
            _ => return Err("unknown or unexpected argument; see `lattice mcp --help`".into()),
        }
    }
    if help {
        return Ok(Parsed::Help);
    }
    let url = url.ok_or("missing --url http://127.0.0.1:PORT/mcp")?;
    let endpoint = check_url(&url)?;
    let token = token.ok_or("missing --token-file FILE")?;
    let token = if token == "-" {
        TokenSource::Stdin
    } else {
        TokenSource::File(token)
    };
    Ok(Parsed::Run(Options { endpoint, token }))
}

/// Reads one line of at most `limit` bytes. A longer line is consumed up to its `\n` without being
/// stored, so memory stays bounded whatever the peer sends.
fn read_framed<R: BufRead>(input: &mut R, limit: usize) -> io::Result<Framed> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut seen = false;
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if too_long {
                Framed::TooLong
            } else if seen {
                Framed::Line(line)
            } else {
                Framed::End
            });
        }
        seen = true;
        let (content, consumed, done) = match chunk.iter().position(|&b| b == b'\n') {
            Some(at) => (chunk.get(..at).unwrap_or_default(), at + 1, true),
            None => (chunk, chunk.len(), false),
        };
        if !too_long {
            if line.len() + content.len() > limit {
                too_long = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(content);
            }
        }
        input.consume(consumed);
        if done {
            break;
        }
    }
    Ok(if too_long {
        Framed::TooLong
    } else {
        Framed::Line(line)
    })
}

/// The token: one line of printable ASCII with no spaces, the line ending not part of it. The
/// error never includes the bytes.
fn token_from(raw: &[u8]) -> Result<Zeroizing<String>, String> {
    let mut body = raw;
    if let Some(rest) = body.strip_suffix(b"\n") {
        body = rest;
    }
    if let Some(rest) = body.strip_suffix(b"\r") {
        body = rest;
    }
    if body.is_empty() || !body.iter().all(u8::is_ascii_graphic) {
        return Err("the token must be one line of printable ASCII with no spaces".into());
    }
    let text = std::str::from_utf8(body).map_err(|_| "the token is not ASCII".to_owned())?;
    Ok(Zeroizing::new(text.to_owned()))
}

/// The id a reply to `message` carries. `None` means `message` is a notification (an object with
/// no `id` member), which gets no reply. Anything else owes a reply, and one that is not a JSON
/// object, or whose id is not a string, number or null, carries a null id.
fn reply_id(message: &[u8]) -> Option<Value> {
    match serde_json::from_slice::<Value>(message) {
        Ok(Value::Object(fields)) => match fields.get("id") {
            None => None,
            Some(id) if id.is_string() || id.is_number() || id.is_null() => Some(id.clone()),
            Some(_) => Some(Value::Null),
        },
        _ => Some(Value::Null),
    }
}

fn error_line(id: &Value, code: i64, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
    .to_string()
}

/// Turns the bytes of a reply into an outcome. Only the framing is read: the head for the status
/// and the length, and the body is passed on as it is.
fn parse_reply(raw: &[u8]) -> Result<Outcome, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the reply has no complete head")?;
    let head = raw.get(..split).unwrap_or_default();
    let body = raw.get(split + 4..).unwrap_or_default();
    let head = std::str::from_utf8(head).map_err(|_| "the reply head is not text".to_owned())?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("the reply status line is malformed")?;
    let mut length: Option<usize> = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("a reply header is malformed")?;
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("the gateway chose a transfer encoding this proxy does not read".into());
        }
        if name.eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "the reply Content-Length is malformed".to_owned())?;
            if length.replace(parsed).is_some() {
                return Err("the reply has two Content-Length headers".into());
            }
        }
    }
    // Required: a reply that stops short is never mistaken for a complete one.
    let len = length.ok_or("the reply has no Content-Length, so it cannot be read in full")?;
    let body = body
        .get(..len)
        .ok_or("the connection closed before the reply was complete")?;
    match status {
        202 => Ok(Outcome::Accepted),
        200 if body.is_empty() => Err("the gateway sent an empty reply".into()),
        200 if body.contains(&b'\n') || body.contains(&b'\r') => {
            Err("the gateway reply has a line break and cannot be written as one line".into())
        }
        200 => Ok(Outcome::Reply(body.to_vec())),
        other => Err(format!("the gateway answered HTTP {other}")),
    }
}

fn send_error(e: io::Error) -> String {
    format!("cannot send to the gateway: {e}")
}

/// Sends one message as `POST /mcp` on a new connection, and reads the whole reply. The token is
/// only ever written to the socket.
fn forward(endpoint: &Endpoint, token: &str, message: &[u8]) -> Result<Outcome, String> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, CONNECT_TIMEOUT).map_err(|e| {
        let hint = if endpoint.localhost {
            " (localhost resolves to 127.0.0.1 only; a gateway bound to [::1] needs http://[::1]:PORT/mcp)"
        } else {
            ""
        };
        format!("cannot reach the gateway at {}: {e}{hint}", endpoint.addr)
    })?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set the read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| format!("cannot set the write timeout: {e}"))?;
    // The head carries the token, so it is built in a buffer that is wiped when it is dropped.
    let length = message.len().to_string();
    let mut head: Zeroizing<Vec<u8>> =
        Zeroizing::new(Vec::with_capacity(256 + endpoint.host.len() + token.len()));
    head.extend_from_slice(b"POST /mcp HTTP/1.1\r\nHost: ");
    head.extend_from_slice(endpoint.host.as_bytes());
    head.extend_from_slice(b"\r\nAuthorization: Bearer ");
    head.extend_from_slice(token.as_bytes());
    head.extend_from_slice(
        b"\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: ",
    );
    head.extend_from_slice(length.as_bytes());
    head.extend_from_slice(b"\r\nConnection: close\r\n\r\n");
    stream.write_all(&head).map_err(send_error)?;
    stream.write_all(message).map_err(send_error)?;
    stream.flush().map_err(send_error)?;
    let mut raw = Vec::new();
    (&mut stream)
        .take(MAX_REPLY_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot read the gateway reply: {e}"))?;
    if raw.len() as u64 > MAX_REPLY_BYTES {
        return Err("the gateway reply is larger than 4 MiB".into());
    }
    parse_reply(&raw)
}

fn write_line<W: Write>(output: &mut W, bytes: &[u8]) -> Result<(), String> {
    let io_error = |e: io::Error| format!("cannot write to standard output: {e}");
    output.write_all(bytes).map_err(io_error)?;
    output.write_all(b"\n").map_err(io_error)?;
    output.flush().map_err(io_error)
}

/// Reads messages until standard input ends. A failed message gets a JSON-RPC error line when it
/// owes a reply, and a line on standard error either way. Only a failure to read or write the
/// standard streams ends the loop with an error.
fn pump<R: BufRead, W: Write>(
    endpoint: &Endpoint,
    token: &str,
    input: &mut R,
    output: &mut W,
) -> Result<(), String> {
    loop {
        let line = match read_framed(input, MAX_LINE_BYTES)
            .map_err(|e| format!("cannot read standard input: {e}"))?
        {
            Framed::End => return Ok(()),
            Framed::TooLong => {
                let refusal = error_line(
                    &Value::Null,
                    INVALID_REQUEST,
                    &format!("a message larger than {MAX_LINE_BYTES} bytes was refused"),
                );
                write_line(output, refusal.as_bytes())?;
                continue;
            }
            Framed::Line(line) => line,
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match forward(endpoint, token, &line) {
            Ok(Outcome::Reply(bytes)) => write_line(output, &bytes)?,
            // A notification is accepted silently. A request is owed a reply, so a 202 for it
            // is reported as an error for its id rather than leaving the client waiting.
            Ok(Outcome::Accepted) => {
                if let Some(id) = reply_id(&line) {
                    let refusal = error_line(&id, INTERNAL_ERROR, "no response from the gateway");
                    write_line(output, refusal.as_bytes())?;
                }
            }
            Err(message) => {
                eprintln!("lattice mcp: {message}");
                if let Some(id) = reply_id(&line) {
                    let refusal = error_line(&id, INTERNAL_ERROR, &message);
                    write_line(output, refusal.as_bytes())?;
                }
            }
        }
    }
}

fn start(opts: Options) -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let token = match &opts.token {
        TokenSource::File(path) => {
            let raw = latticed::files::read_private(Path::new(path), MAX_TOKEN_BYTES)
                .map_err(|e| format!("{path}: {e}"))?;
            token_from(&raw)?
        }
        TokenSource::Stdin => match read_framed(&mut input, MAX_TOKEN_BYTES)
            .map_err(|e| format!("cannot read the token: {e}"))?
        {
            Framed::Line(line) => token_from(&line)?,
            Framed::TooLong => {
                return Err(format!("the token is larger than {MAX_TOKEN_BYTES} bytes"));
            }
            Framed::End => {
                return Err("standard input is empty; the token is its first line".into());
            }
        },
    };
    let stdout = io::stdout();
    let mut output = stdout.lock();
    pump(&opts.endpoint, &token, &mut input, &mut output)
}

/// `lattice mcp ...`: exit 0 for help and for a clean end of standard input, 2 for a bad
/// invocation, 1 when the proxy cannot run or its standard streams fail.
pub(crate) fn command(args: &[String]) -> ExitCode {
    match parse(args) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(opts)) => match start(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("lattice mcp: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("lattice mcp: {e}");
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
    use std::io::Cursor;

    #[test]
    fn framing_keeps_each_line_and_drops_an_oversize_one_as_it_reads() {
        let mut input = Cursor::new(b"abc\nab\n\nxyzw\nc".to_vec());
        assert_eq!(
            read_framed(&mut input, 3).unwrap(),
            Framed::Line(b"abc".to_vec())
        );
        assert_eq!(
            read_framed(&mut input, 3).unwrap(),
            Framed::Line(b"ab".to_vec())
        );
        assert_eq!(
            read_framed(&mut input, 3).unwrap(),
            Framed::Line(Vec::new())
        );
        assert_eq!(read_framed(&mut input, 3).unwrap(), Framed::TooLong);
        assert_eq!(
            read_framed(&mut input, 3).unwrap(),
            Framed::Line(b"c".to_vec())
        );
        assert_eq!(read_framed(&mut input, 3).unwrap(), Framed::End);
    }

    #[test]
    fn a_notification_owes_no_reply_and_a_bad_id_gets_a_null_one() {
        assert_eq!(reply_id(br#"{"jsonrpc":"2.0","method":"x"}"#), None);
        assert_eq!(reply_id(br#"{"id":"a"}"#), Some(Value::String("a".into())));
        assert_eq!(reply_id(br#"{"id":[1]}"#), Some(Value::Null));
        assert_eq!(reply_id(b"not json"), Some(Value::Null));
    }

    #[test]
    fn only_loopback_http_urls_on_the_mcp_path_are_accepted() {
        for ok in [
            "http://127.0.0.1:7000/mcp",
            "http://127.0.0.1:7000",
            "http://localhost:7000/mcp",
            "http://[::1]:7000/mcp",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "https://127.0.0.1:7000/mcp",
            "ws://127.0.0.1:7000/mcp",
            "http://192.0.2.10:7000/mcp",
            "http://[::ffff:127.0.0.1]:7000/mcp",
            "http://127.0.0.1/mcp",
            "http://127.0.0.1:0/mcp",
            "http://127.0.0.1:7000/sync",
            "http://127.0.0.1:7000/mcp?t=1",
            "http://user@127.0.0.1:7000/mcp",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
        let tls = check_url("https://127.0.0.1:7000/mcp").err().unwrap();
        assert!(tls.contains("TLS"), "{tls}");
    }

    #[test]
    fn a_short_or_refused_reply_never_reaches_standard_output_as_text() {
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"a\":").is_err());
        assert_eq!(
            parse_reply(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n").unwrap(),
            Outcome::Accepted
        );
        assert_eq!(
            parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap(),
            Outcome::Reply(b"{}".to_vec())
        );
        let text = "authentication required";
        let refused = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        );
        let message = parse_reply(refused.as_bytes()).err().unwrap();
        assert!(
            message.contains("401") && !message.contains("authentication"),
            "{message}"
        );
    }

    #[test]
    fn a_reply_without_content_length_is_refused_even_when_it_looks_complete() {
        let message = parse_reply(b"HTTP/1.1 200 OK\r\n\r\n{\"id\":1}")
            .err()
            .unwrap();
        assert!(message.contains("Content-Length"), "{message}");
        assert!(parse_reply(b"HTTP/1.1 202 Accepted\r\n\r\n").is_err());
    }

    #[test]
    fn a_200_body_with_a_carriage_return_or_line_feed_is_not_one_line() {
        for body in ["{\r}", "{\n}"] {
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let message = parse_reply(reply.as_bytes()).err().unwrap();
            assert!(message.contains("line break"), "{message}");
        }
    }
}
