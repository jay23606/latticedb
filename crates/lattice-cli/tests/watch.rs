//! `lattice watch` through the real binary. Every check here happens before a socket is opened, so
//! the tests need no gateway.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use std::path::Path;
use std::process::{Command, Output};

fn lattice(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(args)
        .output()
        .unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A token file in a fresh directory of its own, with the given Unix mode.
fn token_file(name: &str, contents: &str, mode: u32) -> String {
    let dir = std::env::temp_dir().join(format!("lattice-watch-{name}-{}", std::process::id()));
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

/// Valid arguments for a watch that needs the token file at `token`.
fn watch_with_token(token: &str) -> Output {
    lattice(&[
        "watch",
        "table/notes",
        "--url",
        "ws://127.0.0.1:7000",
        "--token-file",
        token,
        "--from-start",
    ])
}

#[test]
fn help_lists_the_exact_flags() {
    let o = lattice(&["watch", "--help"]);
    assert_eq!(code(&o), 0, "{}", text(&o.stderr));
    let usage = text(&o.stdout);
    for flag in [
        "table/NAME",
        "--url",
        "--token-file",
        "--from CURSOR",
        "--from-start",
    ] {
        assert!(usage.contains(flag), "{flag} missing from:\n{usage}");
    }
}

#[test]
fn watch_refuses_a_non_loopback_ws_url_without_tls() {
    let token = token_file("url", "tok\n", 0o600);
    let remote = lattice(&[
        "watch",
        "table/notes",
        "--url",
        "ws://192.0.2.10:7000",
        "--token-file",
        &token,
        "--from-start",
    ]);
    assert_eq!(code(&remote), 2);
    assert!(
        text(&remote.stderr).contains("loopback"),
        "{}",
        text(&remote.stderr)
    );

    let tls = lattice(&[
        "watch",
        "table/notes",
        "--url",
        "wss://127.0.0.1:7000",
        "--token-file",
        &token,
        "--from-start",
    ]);
    assert_eq!(code(&tls), 2);
    assert!(text(&tls.stderr).contains("TLS"), "{}", text(&tls.stderr));
}

#[test]
fn watch_refuses_userinfo_uppercase_scheme_and_lookalike_hosts() {
    let token = token_file("lookalike", "tok\n", 0o600);
    for url in [
        "ws://127.0.0.1@evil.com:5433",
        "WS://127.0.0.1:5433/sync",
        "ws://127.0.0.1.evil.com:5433",
    ] {
        let o = lattice(&[
            "watch",
            "table/notes",
            "--url",
            url,
            "--token-file",
            &token,
            "--from-start",
        ]);
        assert_eq!(code(&o), 2, "{url}");
        assert!(!text(&o.stderr).contains("not in this build"), "{url}");
    }
}

#[test]
fn watch_refuses_a_token_on_the_command_line() {
    let o = lattice(&[
        "watch",
        "table/notes",
        "--url",
        "ws://127.0.0.1:7000",
        "--token",
        "s3cret-value",
        "--from-start",
    ]);
    assert_eq!(code(&o), 2);
    assert!(
        text(&o.stderr).contains("command line"),
        "{}",
        text(&o.stderr)
    );
    assert!(!text(&o.stderr).contains("s3cret-value"));
    assert!(!text(&o.stdout).contains("s3cret-value"));
}

#[test]
fn watch_with_valid_arguments_stops_at_the_missing_transport() {
    let token = token_file("valid", "tok\n", 0o600);
    let o = watch_with_token(&token);
    assert_eq!(code(&o), 1, "{}", text(&o.stderr));
    assert!(
        text(&o.stderr).contains("not in this build yet"),
        "{}",
        text(&o.stderr)
    );
}

#[test]
fn watch_with_an_empty_token_file_is_refused_before_anything_else() {
    let token = token_file("empty", "\n", 0o600);
    let o = watch_with_token(&token);
    assert_eq!(code(&o), 1);
    assert!(
        text(&o.stderr).contains("printable ASCII"),
        "{}",
        text(&o.stderr)
    );
}

#[cfg(unix)]
#[test]
fn a_token_file_readable_by_others_is_refused_and_its_contents_stay_out_of_the_message() {
    let token = token_file("mode-0644", "s3cret-tok\n", 0o644);
    let o = watch_with_token(&token);
    assert_eq!(code(&o), 1, "{}", text(&o.stderr));
    let err = text(&o.stderr);
    assert!(err.contains(&token), "{err}");
    assert!(err.contains("0600"), "{err}");
    assert!(!err.contains("s3cret-tok"), "{err}");
    assert!(!text(&o.stdout).contains("s3cret-tok"));
}

#[cfg(unix)]
#[test]
fn a_token_file_with_owner_only_mode_is_accepted() {
    let token = token_file("mode-0600", "tok\n", 0o600);
    let o = watch_with_token(&token);
    assert_eq!(code(&o), 1, "{}", text(&o.stderr));
    assert!(
        text(&o.stderr).contains("not in this build yet"),
        "{}",
        text(&o.stderr)
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_token_file_is_refused() {
    let target = token_file("symlink-target", "tok\n", 0o600);
    let link = Path::new(&target).with_file_name("link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let o = watch_with_token(&link.to_string_lossy());
    assert_eq!(code(&o), 1, "{}", text(&o.stderr));
    assert!(
        !text(&o.stderr).contains("not in this build yet"),
        "{}",
        text(&o.stderr)
    );
}
