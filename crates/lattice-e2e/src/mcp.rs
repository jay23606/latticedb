//! A raw client for `POST /mcp`, the MCP endpoint on the gateway port. The request is hand-written
//! HTTP/1.1 and the reply is read from the socket with its length, so the harness judges what the
//! gateway sent, not the server's own types.

use crate::json::Json;
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// The most bytes of a reply head the client reads before it gives up.
const MAX_HEAD: usize = 16 * 1024;

/// The most body bytes the client reads from one reply (the server caps a reply near 1 MiB).
const MAX_BODY: usize = 4 * 1024 * 1024;

/// How long one socket operation may wait before the request fails.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A client for `POST /mcp` on one gateway address, with an optional bearer token.
#[derive(Clone)]
pub struct Mcp {
    address: SocketAddr,
    token: Option<String>,
}

/// The token is never printed: a formatted client shows only whether it holds one.
impl fmt::Debug for Mcp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let token = if self.token.is_some() {
            "<redacted>"
        } else {
            "none"
        };
        f.debug_struct("Mcp")
            .field("address", &self.address)
            .field("token", &token)
            .finish()
    }
}

/// One HTTP reply from the gateway.
#[derive(Debug)]
pub struct Reply {
    /// The status code of the status line.
    pub status: u16,
    /// The status line and the header lines, each with its CRLF and without the blank line.
    pub head: String,
    /// The body, as text.
    pub body: String,
}

impl Mcp {
    /// A client for the endpoint at `address`. `token` is sent as `Authorization: Bearer`; `None`
    /// sends no credential at all.
    pub fn new(address: SocketAddr, token: Option<&str>) -> Mcp {
        Mcp {
            address,
            token: token.map(str::to_owned),
        }
    }

    /// Posts `body` as the raw JSON-RPC message of one request.
    pub fn post(&self, body: &str) -> Result<Reply, String> {
        // `localhost` with the port is one of the names the gateway allows by default.
        let mut head = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            self.address.port(),
            body.len()
        );
        if let Some(token) = &self.token {
            head.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        head.push_str("\r\n");
        let mut socket = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(TIMEOUT))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(TIMEOUT))
            .map_err(|e| e.to_string())?;
        // A refused request may be answered before its body is sent; the answer still counts, so a
        // failed write is not reported here. A missing answer fails the read below.
        let _ = socket.write_all(head.as_bytes());
        let _ = socket.write_all(body.as_bytes());
        read_reply(&mut socket)
    }

    /// Calls `tool` with `arguments` (a JSON object as text) in the request `id`.
    pub fn call(&self, id: u64, tool: &str, arguments: &str) -> Result<Reply, String> {
        self.post(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
        ))
    }

    /// Lists the tools this token holds a grant for, in the request `id`.
    pub fn list(&self, id: u64) -> Result<Reply, String> {
        self.post(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#
        ))
    }
}

impl Reply {
    /// The body parsed as JSON.
    pub fn json(&self) -> Result<Json, String> {
        Json::parse(&self.body)
    }

    /// The value of a header, matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (field, value) = line.split_once(':')?;
            field
                .trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }
}

/// Reads one reply: the head byte by byte, so nothing past it is consumed, then the body by its
/// `Content-Length`, or to the close of the connection when there is none.
fn read_reply(socket: &mut TcpStream) -> Result<Reply, String> {
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        if raw.len() >= MAX_HEAD {
            return Err("the reply head is too long".into());
        }
        let mut byte = [0u8; 1];
        socket
            .read_exact(&mut byte)
            .map_err(|e| format!("reading the reply head: {e}"))?;
        raw.extend_from_slice(&byte);
    }
    let head = String::from_utf8(raw).map_err(|e| e.to_string())?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("no status line in {head:?}"))?;
    let mut length: Option<usize> = None;
    for line in head.lines().skip(1) {
        if let Some((field, value)) = line.split_once(':')
            && field.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok();
        }
    }
    let body = match length {
        Some(n) => {
            if n > MAX_BODY {
                return Err(format!("a reply body of {n} bytes is over the limit"));
            }
            let mut body = vec![0u8; n];
            socket
                .read_exact(&mut body)
                .map_err(|e| format!("reading the reply body: {e}"))?;
            body
        }
        None => {
            let mut body = Vec::new();
            // `Read::take` names the trait: `TcpStream` implements `Write` too, so a bare method
            // call such as `by_ref` is ambiguous.
            Read::take(&mut *socket, MAX_BODY as u64)
                .read_to_end(&mut body)
                .map_err(|e| format!("reading the reply body: {e}"))?;
            body
        }
    };
    let body = String::from_utf8(body).map_err(|e| e.to_string())?;
    Ok(Reply { status, head, body })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::Mcp;
    use std::net::SocketAddr;

    #[test]
    fn debug_output_never_shows_the_token() {
        // Built at run time, so no token-shaped literal sits in the repository.
        let token = format!("lat_{}", "ab".repeat(20));
        let address = SocketAddr::from(([127, 0, 0, 1], 7070));
        let with = format!("{:?}", Mcp::new(address, Some(token.as_str())));
        let fragment = "ab".repeat(4);
        assert!(!with.contains(token.as_str()), "{with}");
        assert!(!with.contains(fragment.as_str()), "{with}");
        assert!(with.contains("<redacted>"), "{with}");
        let without = format!("{:?}", Mcp::new(address, None));
        assert!(without.contains("none"), "{without}");
    }
}
