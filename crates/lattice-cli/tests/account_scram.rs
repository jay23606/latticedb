//! The credential tool against the real server path: files written by the `lattice` binary are
//! loaded the way `latticed` loads them, and a SCRAM-SHA-256 exchange over certificate-verified TLS
//! succeeds with a client proof computed here from the password alone (RFC 5802 and RFC 7677 steps,
//! written independently of the tool and of `lattice-api`).
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, KeyInit, Mac};
use lattice_api::auth::{Accounts, DecoySecret};
use latticed::{Authentication, ServerConfig, ServerHandle, TlsConfig};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

type Client = StreamOwned<ClientConnection, TcpStream>;

fn directory(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("lattice-account-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Runs the real binary; `stdin` is piped in (never an argument).
fn lattice(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn ok(o: &Output) -> String {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout.clone()).unwrap()
}

/// What `latticed --auth scram-tls` does with the verifier and decoy files, then a listener on
/// loopback with a throwaway certificate.
fn serve(dir: &Path) -> (ServerHandle, Arc<ClientConfig>) {
    let decoy = std::fs::read(dir.join("scram-decoy-secret")).unwrap();
    let accounts = Accounts::parse(
        &std::fs::read_to_string(dir.join("scram-verifiers")).unwrap(),
        DecoySecret::from_bytes(&decoy).unwrap(),
    )
    .unwrap();
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = TlsConfig::from_pem(
        generated.cert.pem().as_bytes(),
        generated.signing_key.serialize_pem().as_bytes(),
    )
    .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(generated.cert.der().clone()).unwrap();
    let client =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    let server = ServerHandle::start(ServerConfig {
        address: SocketAddr::from(([127, 0, 0, 1], 0)),
        authentication: Authentication::ScramTls {
            accounts: Arc::new(accounts),
            tls,
        },
        ..ServerConfig::default()
    })
    .unwrap();
    (server, Arc::new(client))
}

fn connect(address: SocketAddr, config: Arc<ClientConfig>) -> Client {
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(20)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    socket.write_all(&[0, 0, 0, 8, 4, 210, 22, 47]).unwrap();
    let mut reply = [0];
    socket.read_exact(&mut reply).unwrap();
    assert_eq!(reply, *b"S");
    let mut connection =
        ClientConnection::new(config, ServerName::try_from("localhost").unwrap()).unwrap();
    connection.complete_io(&mut socket).unwrap();
    StreamOwned::new(connection, socket)
}
fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    bytes.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}
fn message(stream: &mut impl Read) -> (u8, Vec<u8>) {
    let mut header = [0; 5];
    stream.read_exact(&mut header).unwrap();
    let length = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
    let mut body = vec![0; length - 4];
    stream.read_exact(&mut body).unwrap();
    (header[0], body)
}
fn mac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    m.update(data);
    m.finalize().into_bytes().into()
}

/// A full login. Returns whether the server answered AuthenticationOk after the client checked the
/// server signature it computed from the password.
fn login(address: SocketAddr, config: &Arc<ClientConfig>, user: &str, password: &str) -> bool {
    let mut client = connect(address, config.clone());
    let mut startup = vec![0, 0, 0, 0, 0, 3, 0, 0];
    startup.extend_from_slice(b"user\0");
    startup.extend_from_slice(user.as_bytes());
    startup.extend_from_slice(b"\0\0");
    let length = (startup.len() as u32).to_be_bytes();
    startup[..4].copy_from_slice(&length);
    client.write_all(&startup).unwrap();
    assert_eq!(
        message(&mut client),
        (b'R', b"\0\0\0\x0aSCRAM-SHA-256\0\0".to_vec())
    );
    let first = b"n,,n=,r=independent-client-nonce";
    let mut initial = b"SCRAM-SHA-256\0".to_vec();
    initial.extend_from_slice(&(first.len() as u32).to_be_bytes());
    initial.extend_from_slice(first);
    client.write_all(&frame(b'p', &initial)).unwrap();
    let (tag, payload) = message(&mut client);
    assert_eq!((tag, &payload[..4]), (b'R', &[0, 0, 0, 11][..]));
    let server_first = String::from_utf8(payload[4..].to_vec()).unwrap();
    let attrs: Vec<_> = server_first.split(',').collect();
    let nonce = attrs[0].strip_prefix("r=").unwrap();
    let salt = STANDARD
        .decode(attrs[1].strip_prefix("s=").unwrap())
        .unwrap();
    let iterations: u32 = attrs[2].strip_prefix("i=").unwrap().parse().unwrap();
    assert_eq!(iterations, 4096);
    let final_bare = format!("c=biws,r={nonce}");
    let auth = format!("n=,r=independent-client-nonce,{server_first},{final_bare}");
    let mut salted = [0; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations, &mut salted);
    let client_key = mac(&salted, b"Client Key");
    let signature = mac(&Sha256::digest(client_key), auth.as_bytes());
    let proof: Vec<_> = client_key
        .iter()
        .zip(signature)
        .map(|(a, b)| a ^ b)
        .collect();
    let body = format!("{final_bare},p={}", STANDARD.encode(proof));
    client.write_all(&frame(b'p', body.as_bytes())).unwrap();
    let (tag, payload) = message(&mut client);
    if tag != b'R' {
        return false; // ErrorResponse: wrong password
    }
    let expected = format!(
        "v={}",
        STANDARD.encode(mac(&mac(&salted, b"Server Key"), auth.as_bytes()))
    );
    assert_eq!(&payload[..4], &[0, 0, 0, 12]);
    assert_eq!(&payload[4..], expected.as_bytes(), "server signature");
    assert_eq!(message(&mut client), (b'R', vec![0, 0, 0, 0]));
    true
}

#[test]
fn account_add_writes_a_verifier_the_server_accepts() {
    let dir = directory("scram");
    let d = dir.to_str().unwrap();
    let flags = ok(&lattice(&["account", "init", d], ""));
    assert!(flags.contains("--scram-verifiers") && flags.contains("--scram-decoy-secret"));
    let password = "a password that a person chose";
    ok(&lattice(
        &[
            "account",
            "add",
            d,
            "alice",
            "--read",
            "docs",
            "--password-stdin",
        ],
        &format!("{password}\n"),
    ));
    let added = ok(&lattice(&["account", "add", d, "bob"], ""));
    let generated = added
        .lines()
        .find_map(|l| l.strip_prefix("password (shown once, not stored): "))
        .unwrap()
        .to_string();
    for name in [
        "scram-verifiers",
        "scram-decoy-secret",
        "account-policies",
        "bearer-tokens",
    ] {
        assert_eq!(
            std::fs::metadata(dir.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "{name}"
        );
    }
    assert!(
        !std::fs::read_to_string(dir.join("scram-verifiers"))
            .unwrap()
            .contains(password)
    );
    let (server, config) = serve(&dir);
    let address = server.local_addr();
    assert!(login(address, &config, "alice", password));
    assert!(login(address, &config, "bob", &generated));
    // Each secret works for its own account only; an unknown account fails the same way.
    assert!(!login(address, &config, "alice", &generated));
    assert!(!login(address, &config, "bob", password));
    assert!(!login(
        address,
        &config,
        "alice",
        "a wrong password entirely"
    ));
    assert!(!login(address, &config, "nobody", password));
    server.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_binary_reads_passwords_from_stdin_only_and_exits_nonzero_on_refusal() {
    let dir = directory("binary");
    let d = dir.to_str().unwrap();
    ok(&lattice(&["account", "init", d], ""));
    let help = ok(&lattice(&["account", "--help"], ""));
    assert!(help.contains("--password-stdin"));
    let refused = lattice(
        &["account", "add", d, "carol", "--password", "hunter2hunter2"],
        "",
    );
    assert!(!refused.status.success());
    assert!(!String::from_utf8_lossy(&refused.stderr).contains("hunter2"));
    let weak = lattice(
        &["account", "add", d, "carol", "--password-stdin"],
        "short\n",
    );
    assert!(!weak.status.success());
    assert!(!dir.join("scram-verifiers").exists());
    let again = lattice(&["account", "init", d], "");
    assert!(!again.status.success());
    let _ = std::fs::remove_dir_all(dir);
}
