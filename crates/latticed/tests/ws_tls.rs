//! The sync gateway over TLS (issue #578) with real sockets. Each test generates its own throwaway
//! certificate with rcgen, so no key material is committed. The client is a rustls client that trusts
//! that certificate, and the requests are written here, not with the server's types.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use lattice_api::bearer::{CredentialStore, issue};
use lattice_api::exec_host::{Grant, HostLimits, PolicyTable};
use lattice_api::sync::{self, Message};
use lattice_api::websocket::{self, Opcode};
use lattice_seams::OsRng;
use latticed::ws::{GatewayAuth, GatewayConfig, check_gateway_exposure};
use latticed::{ServerConfig, SqlConfig, SqlServer, TlsConfig};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
/// The WebSocket close opcode.
const CLOSE: u8 = 0x8;

struct Material {
    cert: String,
    key: String,
}

/// A fresh self-signed certificate for `localhost`, generated for this test only.
fn material() -> Material {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    Material {
        cert: generated.cert.pem(),
        key: generated.signing_key.serialize_pem(),
    }
}

struct Fixture {
    server: Option<SqlServer>,
    ws: SocketAddr,
    cert: String,
    token: String,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.shutdown();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A gateway with TLS and one bearer token for `alice`. `tune` changes the gateway settings.
fn start(name: &str, tune: impl FnOnce(&mut GatewayConfig)) -> Fixture {
    let dir = std::env::temp_dir().join(format!("latticed-ws-tls-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (token, entry) = issue("alice", 1, None, &mut OsRng).unwrap();
    let mut store = CredentialStore::empty();
    store.insert(entry).unwrap();
    let material = material();
    let tls = TlsConfig::from_pem(material.cert.as_bytes(), material.key.as_bytes()).unwrap();
    let mut gateway = GatewayConfig {
        address: SocketAddr::from(([127, 0, 0, 1], 0)),
        tls: Some(tls),
        auth: GatewayAuth::Bearer(store),
        slice: Duration::from_millis(10),
        ..GatewayConfig::default()
    };
    tune(&mut gateway);
    let server = SqlServer::start_with_gateway(
        ServerConfig {
            address: SocketAddr::from(([127, 0, 0, 1], 0)),
            ..ServerConfig::default()
        },
        SqlConfig {
            data_dir: dir.clone(),
            policies: PolicyTable::new(Grant::DenyAll),
            limits: HostLimits::default(),
            allow_format_upgrade: false,
        },
        Some(gateway),
    )
    .unwrap();
    Fixture {
        ws: server.gateway_addr().unwrap(),
        server: Some(server),
        cert: material.cert,
        token: token.expose_secret().to_string(),
        dir,
    }
}

fn client_config(cert_pem: &str) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(cert_pem.as_bytes()) {
        roots.add(cert.unwrap()).unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(config)
}

/// A TCP connection wrapped in a TLS client that trusts `cert_pem`. The handshake runs on first use.
fn tls_connect(address: SocketAddr, cert_pem: &str) -> io::Result<Tls> {
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
    let name = ServerName::try_from("localhost").unwrap();
    let conn =
        rustls::ClientConnection::new(client_config(cert_pem), name).map_err(io::Error::other)?;
    Ok(rustls::StreamOwned::new(conn, tcp))
}

/// An upgrade request to `/sync` with the given `Host` header and extra header lines.
fn sync_request(host: &str, extra: &str) -> String {
    format!(
        "GET /sync HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: lattice.sync.v1\r\n{extra}\r\n"
    )
}

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}\r\n")
}

/// Reads one response head byte by byte, so nothing after the blank line is consumed.
fn read_head(stream: &mut impl Read) -> io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn status_of(head: &str) -> u16 {
    head.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Opens TLS and sends the upgrade request with `token`, returning the status and the stream. An
/// error means the handshake or the reply failed (for example, the server refused the connection).
fn open(address: SocketAddr, cert: &str, token: Option<&str>) -> io::Result<(u16, Tls)> {
    let mut tls = tls_connect(address, cert)?;
    let extra = token.map(bearer).unwrap_or_default();
    tls.write_all(sync_request("localhost", &extra).as_bytes())?;
    tls.flush()?;
    let head = read_head(&mut tls)?;
    Ok((status_of(&head), tls))
}

/// Sends an upgrade request as written by the caller and returns only the status.
fn sync_status(address: SocketAddr, cert: &str, host: &str, extra: &str) -> io::Result<u16> {
    let mut tls = tls_connect(address, cert)?;
    tls.write_all(sync_request(host, extra).as_bytes())?;
    tls.flush()?;
    Ok(status_of(&read_head(&mut tls)?))
}

/// Sends a `Hello` as one masked binary WebSocket frame.
fn send_hello(tls: &mut Tls) {
    let mut bytes = Vec::new();
    let hello = Message::Hello {
        min_version: 1,
        max_version: 1,
        agent: "tls-test".into(),
    };
    sync::encode(&hello, &mut bytes).unwrap();
    let mut frame = Vec::new();
    websocket::encode_client_frame(true, Opcode::Binary, &bytes, [1, 2, 3, 4], &mut frame).unwrap();
    tls.write_all(&frame).unwrap();
    tls.flush().unwrap();
}

/// Reads one unmasked server frame and returns its opcode and payload (a small frame: no 64-bit
/// length).
fn read_frame(tls: &mut Tls) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 2];
    tls.read_exact(&mut header)?;
    let len = match header[1] & 0x7f {
        n @ 0..=125 => usize::from(n),
        126 => {
            let mut extended = [0u8; 2];
            tls.read_exact(&mut extended)?;
            usize::from(u16::from_be_bytes(extended))
        }
        _ => panic!("unexpected frame length in a small test frame"),
    };
    let mut payload = vec![0u8; len];
    tls.read_exact(&mut payload)?;
    Ok((header[0] & 0x0f, payload))
}

/// Reads frames until a close frame and returns its status code (`None` if it carries none).
fn read_close_code(tls: &mut Tls) -> io::Result<Option<u16>> {
    for _ in 0..16 {
        let (opcode, payload) = read_frame(tls)?;
        if opcode == CLOSE {
            return Ok(payload.get(..2).map(|c| u16::from_be_bytes([c[0], c[1]])));
        }
    }
    Ok(None)
}

/// One `POST /mcp` over TLS. Returns the status and the body; the server closes after one answer.
fn post_mcp(
    address: SocketAddr,
    cert: &str,
    token: Option<&str>,
    body: &str,
) -> io::Result<(u16, String)> {
    let mut tls = tls_connect(address, cert)?;
    let auth = token.map(bearer).unwrap_or_default();
    let text = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{body}",
        address.port(),
        body.len()
    );
    tls.write_all(text.as_bytes())?;
    tls.flush()?;
    let mut raw = Vec::new();
    // The server ends the session with close_notify; a read error after the answer still counts.
    let _ = tls.read_to_end(&mut raw);
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    Ok((status_of(head), body.to_owned()))
}

/// A `POST /mcp` with the given `Host` and optional `Origin` over TLS; returns only the status.
fn mcp_status(
    address: SocketAddr,
    cert: &str,
    host: &str,
    origin: Option<&str>,
    token: Option<&str>,
) -> io::Result<u16> {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    let auth = token.map(bearer).unwrap_or_default();
    let text = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}\r\n{origin}Content-Type: application/json\r\nContent-Length: {}\r\n{auth}\r\n{body}",
        body.len()
    );
    let mut tls = tls_connect(address, cert)?;
    tls.write_all(text.as_bytes())?;
    tls.flush()?;
    Ok(status_of(&read_head(&mut tls)?))
}

#[test]
fn tls_gateway_upgrades_with_a_bearer_token_and_refuses_without_one() {
    let fixture = start("upgrade", |_| {});
    let (status, mut tls) = open(fixture.ws, &fixture.cert, Some(&fixture.token)).unwrap();
    assert_eq!(status, 101);
    send_hello(&mut tls);
    let (opcode, payload) = read_frame(&mut tls).unwrap();
    assert_eq!(opcode, 0x2, "a sync frame is one binary message");
    assert!(
        matches!(
            sync::decode_with_limit(&payload, 4096),
            Ok(Some((Message::Welcome { version: 1, .. }, _)))
        ),
        "expected Welcome over TLS"
    );
    let (status, _) = open(fixture.ws, &fixture.cert, None).unwrap();
    assert_eq!(status, 401, "a missing token is refused over TLS too");
}

#[test]
fn tls_gateway_serves_post_mcp_with_a_token_and_refuses_without_one() {
    let fixture = start("mcp", |_| {});
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let (status, text) = post_mcp(fixture.ws, &fixture.cert, Some(&fixture.token), body).unwrap();
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("\"tools\""), "{text}");
    let (status, _) = post_mcp(fixture.ws, &fixture.cert, None, body).unwrap();
    assert_eq!(status, 401);
}

#[test]
fn tls_gateway_refuses_a_foreign_host_and_browser_origins() {
    let fixture = start("hosts", |_| {});
    let port = fixture.ws.port();
    let own = format!("localhost:{port}");
    let foreign = format!("evil.example:{port}");
    let token = Some(fixture.token.as_str());
    let cert = fixture.cert.as_str();
    // /mcp answers only to the bound address or localhost on the bound port (DNS rebinding defence).
    let status = mcp_status(fixture.ws, cert, &foreign, None, token).unwrap();
    assert_eq!(status, 403, "a foreign Host is refused");
    // A browser always sends Origin; /mcp refuses any Origin.
    let status = mcp_status(fixture.ws, cert, &own, Some("https://evil.example"), token).unwrap();
    assert_eq!(status, 403, "a browser Origin is refused on /mcp");
    // Control: the same request without Origin and with the own Host is answered.
    let status = mcp_status(fixture.ws, cert, &own, None, token).unwrap();
    assert_eq!(status, 200);
    // /sync refuses any Origin (the default policy), so no browser page can open it.
    let origin = format!("Origin: https://evil.example\r\n{}", bearer(&fixture.token));
    let status = sync_status(fixture.ws, cert, "localhost", &origin).unwrap();
    assert_eq!(status, 403, "a browser Origin is refused on /sync");
    // /sync checks only that Host is present and well formed: it does not compare it with the bound
    // address. Authentication (the token in the Authorization header) is what admits a client here.
    let status = sync_status(fixture.ws, cert, "evil.example", &bearer(&fixture.token)).unwrap();
    assert_eq!(
        status, 101,
        "/sync accepts any well-formed Host with a valid token"
    );
}

#[test]
fn revocation_closes_a_live_tls_connection() {
    let fixture = start("revoke", |_| {});
    let (status, mut tls) = open(fixture.ws, &fixture.cert, Some(&fixture.token)).unwrap();
    assert_eq!(status, 101);
    send_hello(&mut tls);
    let (_, payload) = read_frame(&mut tls).unwrap();
    assert!(matches!(
        sync::decode_with_limit(&payload, 4096),
        Ok(Some((Message::Welcome { .. }, _)))
    ));
    // Every token is revoked at once: the gateway re-checks the live connection and closes it with
    // 1008 (policy violation), without waiting for the revalidation interval.
    let gateway = fixture.server.as_ref().unwrap().gateway().unwrap();
    assert!(gateway.replace_credentials(CredentialStore::empty()));
    assert_eq!(read_close_code(&mut tls).unwrap(), Some(1008));
}

#[test]
fn plaintext_client_on_the_tls_port_is_rejected() {
    let fixture = start("plaintext", |_| {});
    let mut plain = TcpStream::connect_timeout(&fixture.ws, Duration::from_secs(2)).unwrap();
    plain
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    plain
        .write_all(sync_request("localhost", &bearer(&fixture.token)).as_bytes())
        .unwrap();
    let started = Instant::now();
    let mut raw = Vec::new();
    // The TLS handshake fails on the first byte, so the server closes; it never answers in plaintext.
    let _ = plain.read_to_end(&mut raw);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "closed after {:?}",
        started.elapsed()
    );
    assert!(
        !String::from_utf8_lossy(&raw).contains("HTTP/1.1"),
        "no plaintext HTTP reply on the TLS port"
    );
    // The gateway itself is unaffected.
    let (status, _) = open(fixture.ws, &fixture.cert, Some(&fixture.token)).unwrap();
    assert_eq!(status, 101);
}

#[test]
fn stalled_tls_handshake_is_cut_and_frees_its_place() {
    let fixture = start("stall", |gateway| {
        gateway.handshake_timeout = Duration::from_secs(4);
        gateway.max_handshakes = 1;
    });
    // A connection that never starts its TLS handshake takes the only handshake place once the
    // listener has accepted it. The accept loop runs on its own thread, so the test does not see that
    // moment: it probes until a probe is refused. A probe that gets 101 finished its handshake and gave
    // its place back, so it is not a refusal and the probe is repeated.
    let mut stalled = TcpStream::connect_timeout(&fixture.ws, Duration::from_secs(2)).unwrap();
    stalled
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let probing = Instant::now();
    let refused = loop {
        match open(fixture.ws, &fixture.cert, Some(&fixture.token)) {
            Ok((101, _)) => {
                assert!(
                    probing.elapsed() < Duration::from_secs(3),
                    "the stalled connection never took the handshake place"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            other => break other,
        }
    };
    assert!(
        !matches!(refused, Ok((101, _))),
        "the stalled connection holds the only handshake place"
    );
    // The handshake deadline cuts the stalled connection: the peer sees a clean end of stream.
    let started = Instant::now();
    let mut rest = Vec::new();
    let _ = stalled.read_to_end(&mut rest);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "cut after {:?}",
        started.elapsed()
    );
    assert!(rest.is_empty());
    // The place is free again.
    let (status, _) = open(fixture.ws, &fixture.cert, Some(&fixture.token)).unwrap();
    assert_eq!(status, 101);
}

/// The exposure rule the command line and the library both use (`check_gateway_exposure`), with the
/// address and whether TLS and tokens are configured. A non-loopback address needs both.
#[test]
fn non_loopback_bind_without_tls_and_tokens_refuses_to_start() {
    let remote: SocketAddr = "0.0.0.0:7070".parse().unwrap();
    let neither = check_gateway_exposure(remote, false, false).unwrap_err();
    assert!(
        neither.contains("--tls-cert FILE and --tls-key FILE")
            && neither.contains("--gateway-tokens FILE"),
        "{neither}"
    );
    let no_tokens = check_gateway_exposure(remote, true, false).unwrap_err();
    assert!(
        no_tokens.ends_with("missing --gateway-tokens FILE"),
        "{no_tokens}"
    );
    let no_tls = check_gateway_exposure(remote, false, true).unwrap_err();
    assert!(
        no_tls.ends_with("missing --tls-cert FILE and --tls-key FILE"),
        "{no_tls}"
    );
    // The IPv6 unspecified address is not loopback either.
    let unspecified: SocketAddr = "[::]:7070".parse().unwrap();
    assert!(check_gateway_exposure(unspecified, false, false).is_err());
    assert!(check_gateway_exposure(unspecified, true, false).is_err());
    assert!(check_gateway_exposure(unspecified, false, true).is_err());
    // Both TLS and tokens: accepted, on any address.
    assert!(check_gateway_exposure(remote, true, true).is_ok());
    assert!(check_gateway_exposure(unspecified, true, true).is_ok());
}

/// Plaintext and the development principal are allowed only on the loopback addresses the PostgreSQL
/// listener accepts: `127.0.0.0/8` and `::1` (`IpAddr::is_loopback`). An IPv4-mapped address is not
/// loopback, so it needs TLS and tokens like any other non-loopback address.
#[test]
fn plaintext_only_on_the_loopback_addresses_the_pgwire_listener_accepts() {
    for text in ["127.0.0.1:7070", "127.8.9.10:7070", "[::1]:7070"] {
        let address: SocketAddr = text.parse().unwrap();
        assert!(
            check_gateway_exposure(address, false, false).is_ok(),
            "{text}"
        );
    }
    for text in ["[::ffff:127.0.0.1]:7070", "[::ffff:10.0.0.1]:7070"] {
        let address: SocketAddr = text.parse().unwrap();
        assert!(
            check_gateway_exposure(address, false, false).is_err(),
            "{text} without TLS"
        );
        assert!(
            check_gateway_exposure(address, true, false).is_err(),
            "{text} without tokens"
        );
        assert!(
            check_gateway_exposure(address, true, true).is_ok(),
            "{text} with TLS and tokens"
        );
    }
}

/// The development-trust flags (`--auth development-trust --tls-cert --tls-key --gateway-tokens
/// --gateway-listen 0.0.0.0:PORT`) pass the same rule once TLS and tokens are both given.
#[test]
fn development_trust_with_tls_and_tokens_on_a_non_loopback_address_is_allowed() {
    let listen: SocketAddr = "0.0.0.0:7070".parse().unwrap();
    assert!(check_gateway_exposure(listen, true, true).is_ok());
}

#[test]
fn library_refuses_a_non_loopback_gateway_without_tls_or_tokens() {
    let dir = std::env::temp_dir().join(format!("latticed-ws-tls-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (_, entry) = issue("alice", 1, None, &mut OsRng).unwrap();
    let mut store = CredentialStore::empty();
    store.insert(entry).unwrap();
    let material = material();
    let tls = TlsConfig::from_pem(material.cert.as_bytes(), material.key.as_bytes()).unwrap();
    let start_with = |gateway: GatewayConfig| {
        SqlServer::start_with_gateway(
            ServerConfig {
                address: SocketAddr::from(([127, 0, 0, 1], 0)),
                ..ServerConfig::default()
            },
            SqlConfig {
                data_dir: dir.clone(),
                policies: PolicyTable::new(Grant::DenyAll),
                limits: HostLimits::default(),
                allow_format_upgrade: false,
            },
            Some(gateway),
        )
        .err()
        .map(|e| e.to_string())
        .expect("a non-loopback gateway must not start")
    };
    let any = SocketAddr::from(([0, 0, 0, 0], 0));
    let no_tls = start_with(GatewayConfig {
        address: any,
        auth: GatewayAuth::Bearer(store.clone()),
        ..GatewayConfig::default()
    });
    assert!(no_tls.contains("TLS"), "{no_tls}");
    let no_tokens = start_with(GatewayConfig {
        address: any,
        tls: Some(tls),
        ..GatewayConfig::default()
    });
    assert!(no_tokens.contains("bearer tokens"), "{no_tokens}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_loopback_gateway_refuses_to_start_from_the_binary() {
    let dir = std::env::temp_dir().join(format!("latticed-ws-tls-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut child = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .args([
            "--data-dir",
            dir.to_str().unwrap(),
            "--development-policy",
            "allow-all",
            "--gateway-listen",
            "0.0.0.0:0",
            "--stdin-control",
            "off",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("latticed started a non-loopback gateway without TLS or tokens");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!status.success());
    assert!(
        stderr.contains("--tls-cert FILE and --tls-key FILE")
            && stderr.contains("--gateway-tokens FILE"),
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
