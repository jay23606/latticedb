//! The MCP endpoint on the gateway port: `POST /mcp` (issue #527, part 1), with the browser and
//! DNS-rebinding defences of issue #580 that a loopback HTTP endpoint needs.
//!
//! One request carries one MCP JSON-RPC message in its body and gets one answer, and the connection
//! then closes (every response says `Connection: close`). A request is judged in this order, and no
//! refusal names anything the request sent:
//!
//! 1. The target is `/mcp`. A query string is refused (400), so a token never travels in a URL.
//! 2. `Host` is exactly one value and it is this listener's address or `localhost` on the bound port
//!    (403). A page on another name that resolves to loopback is refused (DNS rebinding).
//! 3. No `Origin` header is accepted, `null` included (403). A browser sends one on every cross-origin
//!    request, so no web page can call the endpoint. No CORS header is ever sent.
//! 4. `POST` only (405, `Allow: POST`).
//! 5. `Authorization: Bearer` is verified as on the WebSocket upgrade: the same credential store, the
//!    same uniform 401 and 429, the same failure count per address. The development gateway has no
//!    credential store, so nothing is served over HTTP without a token.
//! 6. `Content-Type` is `application/json` (415). `Content-Length` appears once and there is no
//!    `Transfer-Encoding` (411). A declared length over [`MAX_BODY`] is refused (413) before any body
//!    byte is read.
//! 7. The body is one JSON-RPC message, answered by a fresh `mcp::Server::sessionless` over a backend
//!    bound to the verified principal: 200 with the response, 202 with an empty body for a notification.
//!    Malformed JSON is the JSON-RPC parse error (-32700) inside a 200.
//!
//! The principal is the one the token verified, never one the body names. The session is stateless per
//! request: nothing carries from one request to the next, and `initialize`, `_meta` and a session id are
//! all optional (a `_meta` that is present is still validated; see `docs/mcp-v0.md`).
//!
//! Ordering of the deadlines: the head is read under the gateway's handshake deadline, and the handshake
//! place is released once the token verifies. The body is then read under the same deadline, so a
//! client that authenticates and stops short holds its live-connection place only until that deadline
//! passes, never longer. The body is bounded by [`MAX_BODY`] and the head by `max_head`; the response
//! is written under the response budget. A long `lattice_subscribe` holds its live-connection place
//! (not a handshake place) for up to its `wait_ms`.

use super::{
    AuthRefusal, CLOSE_GRACE, GatewayConfig, Identity, MCP_PATH, Shared, Slot, authorization,
    unix_secs, write_all_by,
};
use crate::mcp_backend::McpBackend;
use lattice_core::bugs::{self, Bug};
use lattice_sdk::mcp::{Caller, Json, Server, parse_json};
use std::io::Read;
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::Ordering;
use std::time::Instant;

/// The largest request body. The declared length is checked against it before any body byte is read.
pub(super) const MAX_BODY: usize = 1 << 20;

/// Header lines a request may carry: the same bound as the WebSocket upgrade.
const MAX_HEADERS: usize = 64;

/// The member of `params._meta` that the planted principal defect reads.
const PRINCIPAL_META: &str = "io.lattice.principal";

const TEXT: &str = "Content-Type: text/plain; charset=utf-8\r\n";
const JSON_TYPE: &str = "Content-Type: application/json\r\n";
const UNAUTHORIZED: &str = "authentication required";

/// Where the bytes of a connection go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// A complete request line for [`MCP_PATH`] on an endpoint that is served.
    Mcp,
    /// Too few bytes to tell: a request line that may still turn out to be `POST`.
    Pending,
    /// Everything else goes to the WebSocket upgrade parser.
    Sync,
}

/// Decides where a connection's bytes go, from the request line alone.
pub(super) fn route(buf: &[u8], enabled: bool) -> Route {
    const POST: &[u8] = b"POST ";
    let Some(line_end) = buf.windows(2).position(|w| w == b"\r\n") else {
        let posting = buf.starts_with(POST) || (buf.len() < POST.len() && POST.starts_with(buf));
        return if enabled && posting {
            Route::Pending
        } else {
            Route::Sync
        };
    };
    let path = buf.get(..line_end).and_then(request_path);
    if enabled && path == Some(MCP_PATH) {
        Route::Mcp
    } else {
        Route::Sync
    }
}

/// The path of a request line (its target without the query string), when it is UTF-8.
fn request_path(line: &[u8]) -> Option<&str> {
    let mut parts = line.split(|b| *b == b' ');
    let _method = parts.next()?;
    let target = std::str::from_utf8(parts.next()?).ok()?;
    Some(target.split_once('?').map_or(target, |(path, _)| path))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadError {
    /// Not a request this endpoint can read: 400.
    Malformed,
    /// More header lines than the bound: 431.
    TooLarge,
}

/// A request head, split but not interpreted. Names are lower-cased; values are trimmed.
struct Head<'a> {
    method: &'a str,
    target: &'a str,
    headers: Vec<(String, &'a str)>,
}

impl<'a> Head<'a> {
    fn parse(text: &'a str) -> Result<Self, HeadError> {
        if !text.is_ascii() {
            return Err(HeadError::Malformed);
        }
        let mut lines = text.split("\r\n");
        let request = lines.next().unwrap_or_default();
        let mut parts = request.split(' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(HeadError::Malformed);
        };
        if version != "HTTP/1.1" || !target.starts_with('/') || !is_token(method) {
            return Err(HeadError::Malformed);
        }
        let mut headers = Vec::new();
        for line in lines {
            if headers.len() >= MAX_HEADERS {
                return Err(HeadError::TooLarge);
            }
            if line.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f) {
                return Err(HeadError::Malformed);
            }
            let Some((name, value)) = line.split_once(':') else {
                return Err(HeadError::Malformed);
            };
            if !is_token(name) {
                return Err(HeadError::Malformed);
            }
            headers.push((
                name.to_ascii_lowercase(),
                value.trim_matches(|c| c == ' ' || c == '\t'),
            ));
        }
        Ok(Self {
            method,
            target,
            headers,
        })
    }

    fn values(&self, name: &str) -> Vec<&'a str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| *v)
            .collect()
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The one value of a header that may appear once; `None` when it is missing or repeated.
fn single<'a>(values: &[&'a str]) -> Option<&'a str> {
    match values {
        [one] => Some(*one),
        _ => None,
    }
}

/// The `Host` values this listener answers to: its own address and `localhost`, on its bound port.
pub(super) fn allowed_hosts(bound: SocketAddr) -> [String; 2] {
    let port = bound.port();
    let ip = match bound.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    [format!("{ip}:{port}"), format!("localhost:{port}")]
}

/// Whether a request's `Host` is one this listener answers to (ASCII case-insensitive).
fn host_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    if bugs::active(Bug::McpHostCheckSkipped) {
        return true;
    }
    host.is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h)))
}

/// Whether a request carrying `origin_headers` `Origin` headers is refused. Any `Origin` is refused.
fn origin_refused(origin_headers: usize) -> bool {
    origin_headers > 0 && !bugs::active(Bug::McpOriginCheckSkipped)
}

/// Whether the `Content-Type` values name exactly `application/json`, optionally with `charset=utf-8`.
fn content_type_ok(values: &[&str]) -> bool {
    let [value] = values else {
        return false;
    };
    let mut parts = value.split(';').map(str::trim);
    let media = parts.next().unwrap_or_default();
    let params_ok = parts.all(|p| p.eq_ignore_ascii_case("charset=utf-8"));
    media.eq_ignore_ascii_case("application/json") && params_ok
}

/// A `Content-Length` value: decimal digits only, at most 20 of them.
fn parse_length(value: &str) -> Option<usize> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Whether a declared body length is refused before any body byte is read.
fn refused_before_reading(declared: usize) -> bool {
    declared > MAX_BODY && !bugs::active(Bug::McpBodyCheckedAfterRead)
}

/// The caller a tool call is served as: the one bound to the verified principal, always. The body
/// never chooses it. (Planted defect: the body's `_meta` names the identity instead.)
pub(super) fn call_caller(bound: &Caller, body: &[u8]) -> Caller {
    if bugs::active(Bug::McpPrincipalFromBody)
        && let Some(id) = named_principal(body)
    {
        return Caller {
            id,
            grants: bound.grants.clone(),
        };
    }
    bound.clone()
}

/// The identity a body names in `params._meta`, when the body parses and names one.
fn named_principal(body: &[u8]) -> Option<String> {
    let root = parse_json(body).ok()?;
    let meta = root.get("params")?.get("_meta")?;
    match meta.get(PRINCIPAL_META)? {
        Json::Str(name) => Some(name.clone()),
        _ => None,
    }
}

/// Answers one `POST /mcp`. `buf` holds the bytes read so far, which start with the request line.
/// Returns when the response is written; the connection is then closed by its owner.
pub(super) fn answer(
    socket: &mut TcpStream,
    buf: Vec<u8>,
    deadline: Instant,
    address: IpAddr,
    shared: &Shared,
    slot: &mut Slot,
) {
    let Some(backends) = shared.mcp.clone() else {
        return;
    };
    let cfg = &shared.cfg;
    let Some((head_bytes, leftover)) = read_head(socket, buf, deadline, shared) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(&head_bytes) else {
        return refuse(socket, cfg, 400, "Bad Request", "malformed request");
    };
    let head = match Head::parse(text) {
        Ok(head) => head,
        Err(HeadError::Malformed) => {
            return refuse(socket, cfg, 400, "Bad Request", "malformed request");
        }
        Err(HeadError::TooLarge) => {
            return refuse(
                socket,
                cfg,
                431,
                "Request Header Fields Too Large",
                "too many header lines",
            );
        }
    };
    if head.target.contains('?') {
        return refuse(
            socket,
            cfg,
            400,
            "Bad Request",
            "the request target must not have a query string; send the token in the Authorization header",
        );
    }

    let allowed = allowed_hosts(shared.bound);
    if !host_allowed(single(&head.values("host")), &allowed) {
        return refuse(
            socket,
            cfg,
            403,
            "Forbidden",
            "the Host header is not this server's address",
        );
    }
    if origin_refused(head.values("origin").len()) {
        return refuse(
            socket,
            cfg,
            403,
            "Forbidden",
            "requests from a browser origin are not accepted",
        );
    }
    if head.method != "POST" {
        return send(
            socket,
            cfg,
            405,
            "Method Not Allowed",
            &format!("Allow: POST\r\n{TEXT}"),
            b"the endpoint accepts POST only",
        );
    }

    let verified = if shared.bearer {
        let store = shared.store();
        shared.gate.authenticate(
            authorization(&head_bytes),
            &store,
            address,
            shared.now_ms(),
            unix_secs(),
        )
    } else {
        // The development gateway holds no credential store, so no token can verify here. Like the
        // WebSocket upgrade on that gateway, nothing is counted against the address: there is no
        // credential to fail, so the failure limiter is never consulted or fed.
        Err(AuthRefusal::Unauthorized)
    };
    let principal = match verified {
        Ok(Identity::Account(principal, _credential)) => principal,
        Ok(Identity::Development) | Err(AuthRefusal::Unauthorized) => {
            return unauthorized(socket, cfg);
        }
        Err(AuthRefusal::Blocked) => {
            return send(
                socket,
                cfg,
                429,
                "Too Many Requests",
                &format!("Retry-After: 60\r\n{TEXT}"),
                b"too many failed attempts",
            );
        }
    };
    // Authenticated: a long poll below holds no handshake place, only its connection place.
    slot.upgraded();

    if !content_type_ok(&head.values("content-type")) {
        return refuse(
            socket,
            cfg,
            415,
            "Unsupported Media Type",
            "the request body must be application/json",
        );
    }
    if !head.values("transfer-encoding").is_empty() {
        return refuse(
            socket,
            cfg,
            411,
            "Length Required",
            "send Content-Length; chunked bodies are not accepted",
        );
    }
    let declared = match head.values("content-length").as_slice() {
        [] => {
            return refuse(
                socket,
                cfg,
                411,
                "Length Required",
                "Content-Length is required",
            );
        }
        [one] => match parse_length(one) {
            Some(n) => n,
            None => {
                return refuse(
                    socket,
                    cfg,
                    400,
                    "Bad Request",
                    "Content-Length must be a number",
                );
            }
        },
        _ => {
            return refuse(
                socket,
                cfg,
                400,
                "Bad Request",
                "Content-Length must appear once",
            );
        }
    };
    if refused_before_reading(declared) {
        return refuse(
            socket,
            cfg,
            413,
            "Payload Too Large",
            "the request body is over 1 MiB",
        );
    }
    let Some(body) = read_body(socket, leftover, declared, deadline, shared) else {
        return;
    };
    if body.len() > MAX_BODY {
        return refuse(
            socket,
            cfg,
            413,
            "Payload Too Large",
            "the request body is over 1 MiB",
        );
    }

    let Some(backend) = backends.backend(&principal) else {
        return refuse(
            socket,
            cfg,
            503,
            "Service Unavailable",
            "the tool backend is not available",
        );
    };
    let caller = call_caller(backend.caller(), &body);
    // One message, no session: `_meta` is optional here, as standard HTTP MCP clients send none.
    let mut server: Server<McpBackend> = Server::sessionless(backend);
    match server.handle(&caller, &body) {
        Some(response) => send(socket, cfg, 200, "OK", JSON_TYPE, response.as_bytes()),
        None => send(socket, cfg, 202, "Accepted", "", b""),
    }
}

/// Reads until the head ends. Returns the head without its blank line and the bytes that followed it.
fn read_head(
    socket: &mut TcpStream,
    mut buf: Vec<u8>,
    deadline: Instant,
    shared: &Shared,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let cfg = &shared.cfg;
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            if end + 4 > cfg.max_head {
                refuse(
                    socket,
                    cfg,
                    431,
                    "Request Header Fields Too Large",
                    "the request head is too large",
                );
                return None;
            }
            let rest = buf.split_off(end + 4);
            buf.truncate(end);
            return Some((buf, rest));
        }
        if buf.len() >= cfg.max_head {
            refuse(
                socket,
                cfg,
                431,
                "Request Header Fields Too Large",
                "the request head is too large",
            );
            return None;
        }
        if shared.stopping.load(Ordering::Acquire) {
            return None;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        socket.set_read_timeout(Some(cfg.slice.min(left))).ok()?;
        match socket.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(chunk.get(..n)?),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return None,
        }
    }
}

/// Reads the body: exactly `declared` bytes, of which `body` already holds the first ones.
fn read_body(
    socket: &mut TcpStream,
    mut body: Vec<u8>,
    declared: usize,
    deadline: Instant,
    shared: &Shared,
) -> Option<Vec<u8>> {
    body.truncate(declared);
    let mut chunk = [0u8; 8192];
    while body.len() < declared {
        if shared.stopping.load(Ordering::Acquire) {
            return None;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        socket
            .set_read_timeout(Some(shared.cfg.slice.min(left)))
            .ok()?;
        let room = (declared - body.len()).min(chunk.len());
        let window = chunk.get_mut(..room)?;
        match socket.read(window) {
            Ok(0) => return None,
            Ok(n) => body.extend_from_slice(chunk.get(..n)?),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return None,
        }
    }
    Some(body)
}

fn refuse(socket: &mut TcpStream, cfg: &GatewayConfig, status: u16, phrase: &str, message: &str) {
    send(socket, cfg, status, phrase, TEXT, message.as_bytes());
}

fn unauthorized(socket: &mut TcpStream, cfg: &GatewayConfig) {
    send(
        socket,
        cfg,
        401,
        "Unauthorized",
        &format!("WWW-Authenticate: Bearer\r\n{TEXT}"),
        UNAUTHORIZED.as_bytes(),
    );
}

/// Writes one response and closes. No CORS header is ever part of it.
fn send(
    socket: &mut TcpStream,
    cfg: &GatewayConfig,
    status: u16,
    phrase: &str,
    headers: &str,
    body: &[u8],
) {
    let head = format!(
        "HTTP/1.1 {status} {phrase}\r\n{headers}Cache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    let deadline = Instant::now() + cfg.response_timeout;
    let _ = write_all_by(socket, &out, cfg.write_timeout, deadline);
    linger(socket, cfg);
}

/// Half-closes, then reads what the client still sends for a short grace, so a response is not
/// destroyed by a reset when the request was refused before its body was read.
fn linger(socket: &mut TcpStream, cfg: &GatewayConfig) {
    let _ = socket.shutdown(Shutdown::Write);
    let until = Instant::now() + CLOSE_GRACE;
    let mut sink = [0u8; 512];
    let _ = socket.set_read_timeout(Some(cfg.slice));
    while Instant::now() < until {
        match socket.read(&mut sink) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn bound() -> SocketAddr {
        "127.0.0.1:5433".parse().unwrap()
    }

    fn allowed() -> [String; 2] {
        allowed_hosts(bound())
    }

    #[test]
    fn the_host_is_this_listener_or_localhost_on_its_port() {
        let allowed = allowed();
        assert!(host_allowed(Some("127.0.0.1:5433"), &allowed));
        assert!(host_allowed(Some("LOCALHOST:5433"), &allowed));
        assert!(host_allowed(Some("localhost:5433"), &allowed));
        assert!(!host_allowed(Some("127.0.0.1:5434"), &allowed));
        assert!(!host_allowed(Some("localhost"), &allowed));
        assert!(!host_allowed(Some("rebind.example:5433"), &allowed));
        assert!(!host_allowed(Some("[::1]:5433"), &allowed));
        assert!(!host_allowed(None, &allowed));
    }

    #[test]
    fn a_planted_host_check_skip_is_caught_by_the_witness() {
        let allowed = allowed();
        bugs::clear();
        bugs::enable(Bug::McpHostCheckSkipped);
        let caught = host_allowed(Some("rebind.example:5433"), &allowed);
        bugs::clear();
        assert!(caught, "the planted bug must accept a foreign Host");
    }

    #[test]
    fn any_origin_is_refused_and_a_planted_skip_is_caught() {
        assert!(!origin_refused(0));
        assert!(origin_refused(1));
        assert!(origin_refused(2));
        bugs::clear();
        bugs::enable(Bug::McpOriginCheckSkipped);
        let caught = !origin_refused(1);
        bugs::clear();
        assert!(caught, "the planted bug must accept a browser Origin");
    }

    #[test]
    fn a_declared_length_over_the_cap_is_refused_before_reading() {
        assert!(!refused_before_reading(0));
        assert!(!refused_before_reading(MAX_BODY));
        assert!(refused_before_reading(MAX_BODY + 1));
        bugs::clear();
        bugs::enable(Bug::McpBodyCheckedAfterRead);
        let caught = !refused_before_reading(MAX_BODY + 1);
        bugs::clear();
        assert!(caught, "the planted bug must read a body it should refuse");
    }

    #[test]
    fn the_call_runs_as_the_bound_caller_whatever_the_body_names() {
        let bound = Caller {
            id: "bob".to_owned(),
            grants: Vec::new(),
        };
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.lattice.principal":"admin"}}}"#;
        assert_eq!(call_caller(&bound, body), bound);
        assert_eq!(call_caller(&bound, b"{not json"), bound);
        bugs::clear();
        bugs::enable(Bug::McpPrincipalFromBody);
        let caught = call_caller(&bound, body).id == "admin";
        bugs::clear();
        assert!(
            caught,
            "the planted bug must take the identity from the body"
        );
    }

    #[test]
    fn the_route_is_decided_from_the_request_line() {
        let post = b"POST /mcp HTTP/1.1\r\nHost: x\r\n";
        assert_eq!(route(post, true), Route::Mcp);
        assert_eq!(route(post, false), Route::Sync);
        assert_eq!(route(b"GET /mcp HTTP/1.1\r\n", true), Route::Mcp);
        assert_eq!(route(b"POST /mcp?t=1 HTTP/1.1\r\n", true), Route::Mcp);
        assert_eq!(route(b"GET /sync HTTP/1.1\r\n", true), Route::Sync);
        assert_eq!(route(b"POST /sync HTTP/1.1\r\n", true), Route::Sync);
        assert_eq!(route(b"POST /mc", true), Route::Pending);
        assert_eq!(route(b"PO", true), Route::Pending);
        assert_eq!(route(b"PO", false), Route::Sync);
        assert_eq!(route(b"PU", true), Route::Sync);
        assert_eq!(route(b"GET /mc", true), Route::Sync);
    }

    #[test]
    fn the_head_is_strict() {
        let ok = Head::parse("POST /mcp HTTP/1.1\r\nHost: a\r\nX-A:  b \t").unwrap();
        assert_eq!(ok.method, "POST");
        assert_eq!(ok.target, "/mcp");
        assert_eq!(ok.values("x-a"), vec!["b"]);
        assert_eq!(
            Head::parse("POST /mcp HTTP/1.0").err(),
            Some(HeadError::Malformed)
        );
        assert_eq!(
            Head::parse("POST  /mcp HTTP/1.1").err(),
            Some(HeadError::Malformed)
        );
        assert_eq!(
            Head::parse("POST /mcp HTTP/1.1\r\nHost : a").err(),
            Some(HeadError::Malformed)
        );
        assert_eq!(
            Head::parse("POST /mcp HTTP/1.1\r\nX: a\u{1}").err(),
            Some(HeadError::Malformed)
        );
        assert_eq!(
            Head::parse("POST /mcp HTTP/1.1\r\nX: é").err(),
            Some(HeadError::Malformed)
        );
        let many: String = (0..65).map(|i| format!("\r\nX-{i}: 1")).collect();
        let text = format!("POST /mcp HTTP/1.1{many}");
        assert_eq!(Head::parse(&text).err(), Some(HeadError::TooLarge));
    }

    #[test]
    fn the_media_type_is_exactly_json() {
        assert!(content_type_ok(&["application/json"]));
        assert!(content_type_ok(&["Application/JSON; charset=UTF-8"]));
        assert!(!content_type_ok(&[]));
        assert!(!content_type_ok(&["text/plain"]));
        assert!(!content_type_ok(&["text/plain;application/json"]));
        assert!(!content_type_ok(&["application/x-www-form-urlencoded"]));
        assert!(!content_type_ok(&["application/json; charset=utf-16"]));
        assert!(!content_type_ok(&["application/json", "application/json"]));
    }

    #[test]
    fn the_length_is_digits_only() {
        assert_eq!(parse_length("0"), Some(0));
        assert_eq!(parse_length("1048577"), Some(1_048_577));
        assert_eq!(parse_length(""), None);
        assert_eq!(parse_length("+1"), None);
        assert_eq!(parse_length("-1"), None);
        assert_eq!(parse_length("1 2"), None);
        assert_eq!(parse_length("123456789012345678901"), None);
    }
}
