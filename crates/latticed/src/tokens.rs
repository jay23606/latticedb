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
use std::path::Path;

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

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use lattice_api::bearer::{STORE_HEADER, issue};
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
}
