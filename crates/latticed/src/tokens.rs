//! The bearer-token credential file on disk (ADR 0031).
//!
//! [`load_credential_store`] opens the file, refuses one that group or others can access or whose
//! directory they can write to, reads at most [`MAX_STORE_BYTES`] and hands the text to the strict
//! parser in `lattice-api`. [`write_credential_store`] replaces the file atomically: a private
//! temporary file in the same directory is written, synced and renamed over the target, then the
//! directory is synced, so a crash leaves the old file or the new one, never a mix. Only Unix mode
//! bits are checked. On other platforms both functions refuse: there is no ACL check yet (#577), and
//! a credential file must not be read unchecked.

use lattice_api::bearer::CredentialStore;
#[cfg(unix)]
use lattice_api::bearer::MAX_STORE_BYTES;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Reads and parses the credential file at `path`.
///
/// The file must be a regular file with no group or other access (mode 0600 or 0400) in a directory
/// that group and others cannot write to. Content errors name a line at most, never content.
pub fn load_credential_store(path: &Path) -> io::Result<CredentialStore> {
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bearer-token files need an owner-only ACL check that this platform does not have yet",
        ))
    }
    #[cfg(unix)]
    {
        let bytes = crate::files::read_private(path, MAX_STORE_BYTES)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "credential file is not UTF-8")
        })?;
        CredentialStore::parse(text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Atomically replaces the credential file at `path` with `store` (mode 0600).
///
/// There must be one writer at a time: the caller serializes concurrent edits.
pub fn write_credential_store(path: &Path, store: &CredentialStore) -> io::Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (path, store);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bearer-token files need an owner-only ACL that this platform does not have yet",
        ))
    }
    #[cfg(unix)]
    {
        let text = store.render();
        if text.len() > MAX_STORE_BYTES {
            return Err(io::Error::other("credential file would be too large"));
        }
        crate::files::replace_private(path, text.as_bytes())
    }
}

/// What one reload check of a [`CredentialWatch`] concluded.
#[derive(Debug)]
pub(crate) enum Reload {
    /// The file loaded: serve this store from now on.
    Loaded(CredentialStore),
    /// The file did not load (first failure of a run of failures). The store in use stays. The error
    /// names a line at most, never a token.
    Kept(io::Error),
    /// The file has not loaded for the whole refusal window: serve no tokens until it does.
    Refused,
}

/// Identity and content stamp of the credential file, taken before it is read. A rename-over (what
/// the writer does) changes the inode, and an in-place edit changes the size or a timestamp.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

#[cfg(unix)]
fn signature(path: &Path) -> io::Result<Signature> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok(Signature {
        dev: metadata.dev(),
        ino: metadata.ino(),
        len: metadata.len(),
        mtime: (metadata.mtime(), metadata.mtime_nsec()),
        ctime: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

#[cfg(not(unix))]
type Signature = ();

#[cfg(not(unix))]
fn signature(_path: &Path) -> io::Result<Signature> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "bearer-token files need an owner-only ACL check that this platform does not have yet",
    ))
}

/// Keeps the bearer-token store in step with its file (ADR 0031, issue #577).
///
/// [`CredentialWatch::check`] runs at most once per `poll`. It reloads when the file's signature
/// changed, when a reload was requested (a SIGHUP count moved), and on every check while the last
/// attempt failed. A file that does not load keeps the store in use; after `refuse_after` of failed
/// attempts the caller must serve no tokens at all. The watch never sees token content, only the
/// parsed result.
#[derive(Debug)]
pub(crate) struct CredentialWatch {
    path: PathBuf,
    poll: Duration,
    refuse_after: Duration,
    next_check: Option<Instant>,
    seen: Option<Signature>,
    requests: u32,
    failing: Option<Failing>,
}

#[derive(Debug, Clone, Copy)]
struct Failing {
    since: Instant,
    refused: bool,
}

impl CredentialWatch {
    /// Loads the file for the first time. The signature is taken before the read, so an edit that
    /// lands during the read is picked up by the next check.
    pub(crate) fn open(
        path: &Path,
        poll: Duration,
        refuse_after: Duration,
        requests: u32,
    ) -> io::Result<(Self, CredentialStore)> {
        let seen = signature(path).ok();
        let store = load_credential_store(path)?;
        let watch = Self {
            path: path.to_path_buf(),
            poll,
            refuse_after,
            next_check: None,
            seen,
            requests,
            failing: None,
        };
        Ok((watch, store))
    }

    /// The file this watch reads.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// One check at `now`. `requests` is the current reload-request count (see
    /// `lattice_sys::signals::reload_signals_received`). Returns `None` when nothing needs doing.
    pub(crate) fn check(&mut self, now: Instant, requests: u32) -> Option<Reload> {
        if self.next_check.is_some_and(|at| now < at) {
            return None;
        }
        self.next_check = Some(now + self.poll);
        let forced = requests != self.requests;
        self.requests = requests;
        let current = signature(&self.path).ok();
        if !forced && self.failing.is_none() && current == self.seen {
            return None;
        }
        self.seen = current;
        match load_credential_store(&self.path) {
            Ok(store) => {
                self.failing = None;
                Some(Reload::Loaded(store))
            }
            Err(error) => match &mut self.failing {
                None => {
                    self.failing = Some(Failing {
                        since: now,
                        refused: false,
                    });
                    Some(Reload::Kept(error))
                }
                Some(failing) => {
                    let stale = now.saturating_duration_since(failing.since) >= self.refuse_after;
                    if stale && !failing.refused {
                        failing.refused = true;
                        Some(Reload::Refused)
                    } else {
                        None
                    }
                }
            },
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use lattice_api::bearer::{
        AuthError, BearerToken, RejectReason, STORE_HEADER, TokenStatus, issue, verify_bearer,
    };
    use lattice_seams::Rng;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    struct Counter(u64);
    impl Rng for Counter {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            self.0
        }
    }

    fn directory(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("latticed-tokens-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
    fn one_token_store() -> CredentialStore {
        let (_, entry) = issue("alice", 10, None, &mut Counter(1)).unwrap();
        let mut store = CredentialStore::empty();
        store.insert(entry).unwrap();
        store
    }

    #[test]
    fn a_written_file_is_private_loads_back_and_leaves_no_temporary_file() {
        let dir = directory("roundtrip");
        let path = dir.join("tokens");
        let store = one_token_store();
        write_credential_store(&path, &store).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            load_credential_store(&path).unwrap().render(),
            store.render()
        );
        // Replacing a file that someone made world-readable gives the new file mode 0600.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_credential_store(&path, &CredentialStore::empty()).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert!(load_credential_store(&path).unwrap().is_empty());
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["tokens"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_removes_its_temporary() {
        let dir = directory("failed");
        let path = dir.join("tokens");
        write_credential_store(&path, &one_token_store()).unwrap();
        let before = std::fs::read(&path).unwrap();
        // A directory in the way of the rename makes the replacement fail after the data was written.
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("inside"), b"x").unwrap();
        assert!(write_credential_store(&blocked, &CredentialStore::empty()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["blocked", "tokens"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn credential_file_with_broader_access_is_refused() {
        let dir = directory("modes");
        let path = dir.join("tokens");
        std::fs::write(&path, format!("{STORE_HEADER}\n")).unwrap();
        for (mode, ok) in [
            (0o600, true),
            (0o400, true),
            (0o640, false),
            (0o604, false),
            (0o660, false),
            (0o644, false),
            (0o666, false),
        ] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(load_credential_store(&path).is_ok(), ok, "file {mode:o}");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        for (mode, ok) in [
            (0o700, true),
            (0o755, true),
            (0o770, false),
            (0o757, false),
            (0o777, false),
        ] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                load_credential_store(&path).is_ok(),
                ok,
                "directory {mode:o}"
            );
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_oversize_non_regular_and_malformed_files_are_refused() {
        let dir = directory("refused");
        let path = dir.join("tokens");
        assert!(load_credential_store(&path).is_err());
        let write = |bytes: &[u8]| {
            let _ = std::fs::remove_file(&path);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        write(&vec![b'x'; MAX_STORE_BYTES + 1]);
        assert!(
            load_credential_store(&path)
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        write(&[0xff, 0xfe]);
        assert!(
            load_credential_store(&path)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        write(b"not a credential file\n");
        assert!(load_credential_store(&path).is_err());
        // A directory is not a credential file.
        assert!(load_credential_store(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fifo_or_symlink_at_the_path_is_refused_without_blocking() {
        let dir = directory("fifo");
        let fifo = dir.join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(load_credential_store(&fifo).is_err());
        let real = dir.join("real");
        std::fs::write(&real, format!("{STORE_HEADER}\n")).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(load_credential_store(&real).is_ok());
        assert!(load_credential_store(&link).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_writer_checks_the_directory_and_never_touches_other_files() {
        let dir = directory("writer");
        let path = dir.join("tokens");
        let other = dir.join("tokens.tmp1-0");
        std::fs::write(&other, b"not ours").unwrap();
        write_credential_store(&path, &one_token_store()).unwrap();
        write_credential_store(&path, &one_token_store()).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"not ours");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(write_credential_store(&path, &CredentialStore::empty()).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_name_no_file_content() {
        let dir = directory("content");
        let path = dir.join("tokens");
        let secret_looking = format!("lat_0001020304050607_{}", "A".repeat(43));
        std::fs::write(&path, format!("{STORE_HEADER}\n{secret_looking}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let error = load_credential_store(&path).unwrap_err().to_string();
        assert_eq!(error, "credential file line 2 is malformed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn one_token_file(path: &Path, account: &str) -> (BearerToken, CredentialStore) {
        let (token, entry) = issue(account, 10, None, &mut Counter(1)).unwrap();
        let mut store = CredentialStore::empty();
        store.insert(entry).unwrap();
        write_credential_store(path, &store).unwrap();
        (token, store)
    }

    #[test]
    fn a_changed_file_is_loaded_at_the_first_check_after_the_poll_interval() {
        let dir = directory("watch-change");
        let path = dir.join("tokens");
        let (token, mut store) = one_token_file(&path, "alice");
        let poll = Duration::from_millis(100);
        let t0 = Instant::now();
        let (mut watch, loaded) =
            CredentialWatch::open(&path, poll, Duration::from_secs(600), 0).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(
            watch.check(t0, 0).is_none(),
            "an unchanged file is not read"
        );
        store.revoke(token.id());
        write_credential_store(&path, &store).unwrap();
        assert!(
            watch.check(t0 + poll / 2, 0).is_none(),
            "nothing is read before the poll interval"
        );
        match watch.check(t0 + poll, 0) {
            Some(Reload::Loaded(reloaded)) => {
                let entry = reloaded.get(token.id()).unwrap();
                assert_eq!(entry.status(), TokenStatus::Revoked);
            }
            other => panic!("expected the revocation to load, got {other:?}"),
        }
        assert!(watch.check(t0 + poll * 2, 0).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_file_or_a_broader_mode_keeps_the_store_and_is_retried_without_repeating_the_report() {
        let dir = directory("watch-keep");
        let path = dir.join("tokens");
        let (_, store) = one_token_file(&path, "alice");
        let poll = Duration::from_millis(10);
        let t0 = Instant::now();
        let (mut watch, _) =
            CredentialWatch::open(&path, poll, Duration::from_secs(600), 0).unwrap();
        assert!(watch.check(t0, 0).is_none());
        // A partial write: the last newline is missing, so the file is refused.
        let good = store.render();
        std::fs::write(&path, &good.as_bytes()[..good.len() - 1]).unwrap();
        assert!(matches!(watch.check(t0 + poll, 0), Some(Reload::Kept(_))));
        // Still broken: retried at every check, reported once.
        assert!(watch.check(t0 + poll * 2, 0).is_none());
        // Whole again but group-readable: refused by the mode check, so the store still stays.
        std::fs::write(&path, good.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(watch.check(t0 + poll * 3, 0).is_none());
        // Owner-only again: the next check loads the same content.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            watch.check(t0 + poll * 4, 0),
            Some(Reload::Loaded(reloaded)) if reloaded.len() == 1
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_broken_past_the_refusal_window_refuses_every_token_once_then_loads_again() {
        let dir = directory("watch-refuse");
        let path = dir.join("tokens");
        let (_, store) = one_token_file(&path, "alice");
        let poll = Duration::from_millis(10);
        let t0 = Instant::now();
        let (mut watch, _) =
            CredentialWatch::open(&path, poll, Duration::from_millis(50), 0).unwrap();
        assert!(watch.check(t0, 0).is_none());
        std::fs::write(&path, b"not a credential file\n").unwrap();
        assert!(matches!(watch.check(t0 + poll, 0), Some(Reload::Kept(_))));
        // 30 ms of failure: still inside the window.
        assert!(watch.check(t0 + Duration::from_millis(40), 0).is_none());
        // 50 ms since the first failure: refuse, once.
        assert!(matches!(
            watch.check(t0 + Duration::from_millis(60), 0),
            Some(Reload::Refused)
        ));
        assert!(watch.check(t0 + Duration::from_millis(80), 0).is_none());
        write_credential_store(&path, &store).unwrap();
        assert!(matches!(
            watch.check(t0 + Duration::from_millis(100), 0),
            Some(Reload::Loaded(_))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reload_request_rereads_an_unchanged_file_once() {
        let dir = directory("watch-hup");
        let path = dir.join("tokens");
        let _ = one_token_file(&path, "alice");
        let poll = Duration::from_millis(10);
        let t0 = Instant::now();
        let (mut watch, _) =
            CredentialWatch::open(&path, poll, Duration::from_secs(600), 5).unwrap();
        assert!(watch.check(t0, 5).is_none());
        assert!(matches!(watch.check(t0 + poll, 6), Some(Reload::Loaded(_))));
        assert!(watch.check(t0 + poll * 2, 6).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotation_allows_two_tokens_for_a_bounded_overlap_then_one() {
        let dir = directory("rotation-file");
        let path = dir.join("tokens");
        let mut store = CredentialStore::empty();
        let (old, old_entry) = issue("alice", 10, None, &mut Counter(1)).unwrap();
        store.insert(old_entry).unwrap();
        // The rotation ends the older token at 500 s; the newer one is created at 50 s.
        store.set_expiry(old.id(), 500).unwrap();
        let (new, new_entry) = issue("alice", 50, None, &mut Counter(7)).unwrap();
        store.insert(new_entry).unwrap();
        let (_, third) = issue("alice", 60, None, &mut Counter(9)).unwrap();
        assert!(
            store.insert(third).is_err(),
            "a third active token is refused"
        );
        write_credential_store(&path, &store).unwrap();
        let poll = Duration::from_millis(10);
        let t0 = Instant::now();
        let (mut watch, loaded) =
            CredentialWatch::open(&path, poll, Duration::from_secs(600), 0).unwrap();
        let header = |t: &BearerToken| format!("Bearer {}", t.expose_secret());
        // During the overlap both tokens authenticate as alice.
        for token in [&old, &new] {
            let who = verify_bearer(Some(header(token).as_bytes()), &loaded, 100).unwrap();
            assert_eq!(who.name(), "alice");
        }
        // At the older token's expiry only the newer one does.
        assert_eq!(
            verify_bearer(Some(header(&old).as_bytes()), &loaded, 500),
            Err(AuthError::Rejected(RejectReason::Expired))
        );
        assert!(verify_bearer(Some(header(&new).as_bytes()), &loaded, 500).is_ok());
        // Revoking the older one in the file takes effect at the next reload.
        store.revoke(old.id());
        write_credential_store(&path, &store).unwrap();
        let reloaded = match watch.check(t0, 0) {
            Some(Reload::Loaded(reloaded)) => reloaded,
            other => panic!("expected a reload, got {other:?}"),
        };
        assert_eq!(
            verify_bearer(Some(header(&old).as_bytes()), &reloaded, 100),
            Err(AuthError::Rejected(RejectReason::Revoked))
        );
        let active = reloaded
            .entries()
            .filter(|e| e.account() == "alice" && e.status() == TokenStatus::Active)
            .count();
        assert_eq!(active, 1, "one token is left after the overlap");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
