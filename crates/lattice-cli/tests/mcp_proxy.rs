//! `lattice mcp` through the real binary, against a stub gateway on 127.0.0.1 with an ephemeral
//! port. Nothing leaves loopback, and no test needs a real gateway.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};

const TOKEN: &str = "test-token-0123";
/// The gateway's body limit, which the proxy's line limit matches.
const LIMIT: usize = 1 << 20;
const REQUEST_1: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
const REPLY_1: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn lines(o: &Output) -> Vec<String> {
    text(&o.stdout).lines().map(str::to_owned).collect()
}

/// A token file in a fresh directory of its own, with the given Unix mode.
fn token_file(name: &str, contents: &str, mode: u32) -> String {
    let dir = std::env::temp_dir().join(format!("lattice-mcp-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("token");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = mode;
    path.to_string_lossy().into_owned()
}

/// Runs `lattice mcp ARGS` with `input` on standard input, and waits for it to exit.
fn proxy(args: &[&str], input: Vec<u8>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().unwrap();
    let _ = writer.join();
    output
}

/// What the stub gateway received in one request.
struct Request {
    head: String,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }
}

fn read_request(stream: &mut TcpStream) -> Request {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut head = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
        head.push_str(&line);
    }
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    Request { head, body }
}

/// A stub gateway that answers one connection per entry of `replies`, in order, with that raw
/// response, and reports every request it reads. The connection is closed after each reply.
fn stub(replies: Vec<Vec<u8>>) -> (u16, Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let request = read_request(&mut stream);
            let _ = sender.send(request);
            let _ = stream.write_all(&reply);
        }
    });
    (port, receiver)
}

fn http(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

#[test]
fn help_lists_the_exact_flags() {
    let o = proxy(&["--help"], Vec::new());
    assert_eq!(code(&o), 0, "{}", text(&o.stderr));
    let usage = text(&o.stdout);
    for flag in ["--url", "--token-file", "/mcp"] {
        assert!(usage.contains(flag), "{flag} missing from:\n{usage}");
    }
}

#[test]
fn each_line_is_one_post_and_each_reply_is_one_line() {
    let notice = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let reply_2 = r#"{"jsonrpc":"2.0","id":2,"result":{}}"#;
    let (port, requests) = stub(vec![
        http("200 OK", REPLY_1),
        http("202 Accepted", ""),
        http("200 OK", reply_2),
    ]);
    let token = token_file("framing", &format!("{TOKEN}\n"), 0o600);
    let input = format!(
        "{REQUEST_1}\n{notice}\n{}\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#
    );
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        input.into_bytes(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(lines(&out), vec![REPLY_1.to_owned(), reply_2.to_owned()]);
    let seen: Vec<Request> = requests.try_iter().collect();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].body, REQUEST_1.as_bytes());
    assert_eq!(seen[1].body, notice.as_bytes());
    let first = &seen[0];
    assert_eq!(first.head.lines().next(), Some("POST /mcp HTTP/1.1"));
    assert_eq!(
        first.header("Authorization").as_deref(),
        Some("Bearer test-token-0123")
    );
    assert_eq!(
        first.header("Host").as_deref(),
        Some(format!("127.0.0.1:{port}").as_str())
    );
    assert_eq!(
        first.header("Content-Type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        first.header("Content-Length").as_deref(),
        Some(REQUEST_1.len().to_string().as_str())
    );
    assert!(first.header("Origin").is_none());
    assert!(first.header("Transfer-Encoding").is_none());
}

#[test]
fn a_message_larger_than_one_read_is_forwarded_whole() {
    let pad = "x".repeat(300 * 1024);
    let request =
        format!(r#"{{"jsonrpc":"2.0","id":9,"method":"ping","params":{{"pad":"{pad}"}}}}"#);
    let reply = r#"{"jsonrpc":"2.0","id":9,"result":{}}"#;
    let (port, requests) = stub(vec![http("200 OK", reply)]);
    let token = token_file("large", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        format!("{request}\n").into_bytes(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(lines(&out), vec![reply.to_owned()]);
    let seen: Vec<Request> = requests.try_iter().collect();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, request.as_bytes());
}

#[test]
fn an_oversize_line_is_refused_and_the_next_line_still_goes_through() {
    let (port, requests) = stub(vec![http("200 OK", REPLY_1)]);
    let token = token_file("oversize", &format!("{TOKEN}\n"), 0o600);
    let mut input = vec![b'a'; LIMIT + 1];
    input.push(b'\n');
    input.extend_from_slice(REQUEST_1.as_bytes());
    input.push(b'\n');
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        input,
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let got = lines(&out);
    assert_eq!(got.len(), 2, "{got:?}");
    let refusal: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
    assert_eq!(refusal["error"]["code"], -32600);
    assert!(refusal["id"].is_null(), "{refusal}");
    assert_eq!(got[1], REPLY_1);
    let seen: Vec<Request> = requests.try_iter().collect();
    assert_eq!(
        seen.len(),
        1,
        "the oversize bytes must not reach the gateway"
    );
    assert_eq!(seen[0].body, REQUEST_1.as_bytes());
}

#[test]
fn a_token_on_the_command_line_is_refused_without_echoing_it() {
    for extra in [vec!["--token", "s3cret"], vec!["--token=s3cret"]] {
        let mut args = vec!["--url", "http://127.0.0.1:9/mcp"];
        args.extend(extra);
        let out = proxy(&args, Vec::new());
        assert_eq!(code(&out), 2);
        let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
        assert!(all.contains("command line"), "{all}");
        assert!(!all.contains("s3cret"), "{all}");
    }
}

#[test]
fn a_world_readable_token_file_is_refused() {
    let token = token_file("mode", &format!("{TOKEN}\n"), 0o644);
    let out = proxy(
        &[
            "--url",
            "http://127.0.0.1:9/mcp",
            "--token-file",
            token.as_str(),
        ],
        format!("{REQUEST_1}\n").into_bytes(),
    );
    assert_eq!(code(&out), 1);
    assert!(out.stdout.is_empty(), "{}", text(&out.stdout));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(!all.contains(TOKEN), "{all}");
}

#[test]
fn a_non_loopback_or_tls_url_is_refused_before_anything_is_read() {
    for (u, needle) in [
        ("http://192.0.2.10:7000/mcp", "loopback"),
        ("http://example.test:7000/mcp", "loopback"),
        ("https://127.0.0.1:7000/mcp", "TLS"),
        ("http://127.0.0.1:7000/sync", "/mcp"),
    ] {
        let out = proxy(
            &["--url", u, "--token-file", "/nonexistent/token"],
            Vec::new(),
        );
        assert_eq!(code(&out), 2, "{u}");
        assert!(
            text(&out.stderr).contains(needle),
            "{u}: {}",
            text(&out.stderr)
        );
    }
}

#[test]
fn a_reply_is_passed_through_byte_for_byte() {
    let body = r#"{"jsonrpc":"2.0", "id":7,"result":{"text":"café  \t tab"}}"#;
    let (port, _) = stub(vec![http("200 OK", body)]);
    let token = token_file("passthrough", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(out.stdout, format!("{body}\n").into_bytes());
}

#[test]
fn a_stub_that_closes_mid_response_yields_a_json_rpc_error_line() {
    let partial = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"jsonrpc\"".to_vec();
    let (port, _) = stub(vec![partial]);
    let token = token_file("midway", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let got = lines(&out);
    assert_eq!(got.len(), 1, "{got:?}");
    let error: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
    assert_eq!(error["id"], 3);
    assert_eq!(error["error"]["code"], -32603);
}

#[test]
fn a_refused_status_becomes_an_error_line_without_the_server_text() {
    let (port, _) = stub(vec![http("401 Unauthorized", "authentication required")]);
    let token = token_file("refused", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let got = lines(&out);
    assert_eq!(got.len(), 1, "{got:?}");
    let error: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
    assert_eq!(error["id"], 4);
    assert!(error["error"]["message"].as_str().unwrap().contains("401"));
    assert!(!text(&out.stdout).contains("authentication required"));
}

#[test]
fn the_token_can_come_from_the_first_line_of_standard_input() {
    let (port, requests) = stub(vec![http("200 OK", REPLY_1)]);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", "-"],
        format!("{TOKEN}\n{REQUEST_1}\n").into_bytes(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert_eq!(lines(&out), vec![REPLY_1.to_owned()]);
    let seen: Vec<Request> = requests.try_iter().collect();
    assert_eq!(
        seen[0].header("Authorization").as_deref(),
        Some("Bearer test-token-0123")
    );
}

#[test]
fn an_empty_standard_input_exits_cleanly_without_a_connection() {
    let token = token_file("empty", &format!("{TOKEN}\n"), 0o600);
    let out = proxy(
        &[
            "--url",
            "http://127.0.0.1:9/mcp",
            "--token-file",
            token.as_str(),
        ],
        Vec::new(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(out.stdout.is_empty());
}

#[test]
fn a_request_answered_with_202_gets_an_error_line_for_its_id() {
    let (port, _) = stub(vec![http("202 Accepted", "")]);
    let token = token_file("accepted", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let got = lines(&out);
    assert_eq!(got.len(), 1, "{got:?}");
    let error: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
    assert_eq!(error["id"], 5);
    assert_eq!(error["error"]["code"], -32603);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no response")
    );
}

#[test]
fn a_reply_without_content_length_is_an_error_line_not_a_guess() {
    let reply =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":6,\"result\":{}}"
            .to_vec();
    let (port, _) = stub(vec![reply]);
    let token = token_file("nolength", &format!("{TOKEN}\n"), 0o600);
    let u = url(port);
    let out = proxy(
        &["--url", u.as_str(), "--token-file", token.as_str()],
        b"{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"ping\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let got = lines(&out);
    assert_eq!(got.len(), 1, "{got:?}");
    let error: serde_json::Value = serde_json::from_str(&got[0]).unwrap();
    assert_eq!(error["id"], 6);
    assert_eq!(error["error"]["code"], -32603);
    assert!(!text(&out.stdout).contains("\"result\""));
}

#[test]
fn a_localhost_gateway_that_is_down_says_it_connects_to_127_0_0_1() {
    let token = token_file("localhost", &format!("{TOKEN}\n"), 0o600);
    // Port 1 on loopback is closed here; the connect fails and the hint is named.
    let out = proxy(
        &[
            "--url",
            "http://localhost:1/mcp",
            "--token-file",
            token.as_str(),
        ],
        b"{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}\n".to_vec(),
    );
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("127.0.0.1 only"), "{all}");
}
