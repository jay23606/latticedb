//! The owner-only file that holds the saved feed cursor of `lattice watch --cursor-file`.
//!
//! The bytes are the SDK codec's (`lattice_sdk::cursor_store`), which accepts one canonical cursor
//! and nothing else. The file is written with the credential rules of `latticed::files`: a
//! temporary 0600 file is synced and renamed over the old one, then the directory is synced, so a
//! crash leaves the old cursor or the new one, never a mix. The directory must exist and must not be
//! writable by group or others, and a file that group or others can read is refused. A lock file
//! next to the cursor keeps a second watch from saving over the first.

use lattice_sdk::cursor_store::{CursorStore, MalformedCursor, decode_cursor, encode_cursor};
use lattice_sdk::mcp::FeedCursor;
use latticed::files::{read_private, replace_private};
use std::fmt;
use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// The largest cursor file read: 24 digits and one newline. Anything longer is not a cursor.
const MAX_CURSOR_FILE: usize = 64;

/// Why the cursor file could not be loaded, saved or locked.
#[derive(Debug)]
pub(super) enum CursorFileError {
    /// The file exists but does not hold exactly one saved cursor.
    Malformed(MalformedCursor),
    /// The file cannot be read, written or locked. The text names the file and the rule that refused it.
    Io(String),
}

impl From<MalformedCursor> for CursorFileError {
    fn from(error: MalformedCursor) -> Self {
        Self::Malformed(error)
    }
}

impl fmt::Display for CursorFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "{error}"),
            Self::Io(text) => f.write_str(text),
        }
    }
}

/// The saved cursor at one path. A missing file means nothing is saved yet.
#[derive(Debug)]
pub(super) struct FileCursorStore {
    path: PathBuf,
}

/// The lock of one cursor file. It is held for as long as this value lives.
#[derive(Debug)]
pub(super) struct CursorLock {
    _file: File,
}

impl FileCursorStore {
    /// Opens the store for `path`. The directory must exist, be a directory, and not be writable by
    /// group or others: the save replaces the file through that directory.
    pub(super) fn open(path: &Path) -> Result<Self, CursorFileError> {
        let store = Self {
            path: path.to_path_buf(),
        };
        let dir = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let metadata = std::fs::metadata(dir).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => store.refusal("the directory does not exist"),
            _ => store.io_error(error),
        })?;
        if !metadata.is_dir() {
            return Err(store.refusal("its directory is not a directory"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(
                    store.refusal("its directory is writable by group or others (use mode 0700)")
                );
            }
        }
        Ok(store)
    }

    /// Takes the lock of this cursor file for the rest of the watch. The lock lives in `PATH.lock`
    /// and never waits: a second watch on the same file is refused at once.
    pub(super) fn lock(&self) -> Result<CursorLock, CursorFileError> {
        let mut name = self.path.clone().into_os_string();
        name.push(".lock");
        let lock_path = PathBuf::from(name);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| CursorFileError::Io(format!("{}: {error}", lock_path.display())))?;
        match file.try_lock() {
            Ok(()) => Ok(CursorLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(self
                .refusal("another watch holds this cursor file; stop it before starting another")),
            Err(TryLockError::Error(error)) => Err(self.io_error(error)),
        }
    }

    fn io_error(&self, error: io::Error) -> CursorFileError {
        CursorFileError::Io(format!("{}: {error}", self.path.display()))
    }

    fn refusal(&self, reason: &str) -> CursorFileError {
        CursorFileError::Io(format!("{}: {reason}", self.path.display()))
    }
}

impl CursorStore for FileCursorStore {
    type Error = CursorFileError;

    fn load(&self) -> Result<Option<FeedCursor>, CursorFileError> {
        match read_private(&self.path, MAX_CURSOR_FILE) {
            Ok(bytes) => Ok(Some(decode_cursor(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(self.io_error(error)),
        }
    }

    fn save(&mut self, cursor: &FeedCursor) -> Result<(), CursorFileError> {
        replace_private(&self.path, encode_cursor(cursor).as_bytes())
            .map_err(|error| self.io_error(error))
    }
}

#[cfg(all(test, unix))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// LSN 0x102, index 1: the cursor that `formats/change-feed-v1.md` gives as an example.
    const FIRST: FeedCursor = [0, 0, 0, 0, 0, 0, 1, 2, 0, 0, 0, 1];
    const SECOND: FeedCursor = [0, 0, 0, 0, 0, 0, 1, 2, 0, 0, 0, 2];

    fn directory(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lattice-cursor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Writes `bytes` and gives the file mode 0600, so that only its contents differ from a saved
    /// cursor.
    fn write_owner_only(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn a_missing_file_means_no_cursor_is_saved() {
        let dir = directory("missing");
        let store = FileCursorStore::open(&dir.join("cursor")).unwrap();
        assert_eq!(store.load().unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_directory_is_refused_when_the_store_is_opened() {
        let err =
            FileCursorStore::open(Path::new("/nonexistent-lattice-cursor-dir/cursor")).unwrap_err();
        assert!(
            err.to_string().contains("the directory does not exist"),
            "{err}"
        );
    }

    #[test]
    fn a_saved_cursor_loads_back_and_the_file_is_owner_only() {
        let dir = directory("round-trip");
        let path = dir.join("cursor");
        let mut store = FileCursorStore::open(&path).unwrap();
        store.save(&FIRST).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "000000000000010200000001\n"
        );
        assert_eq!(store.load().unwrap(), Some(FIRST));

        // The second save replaces the first and leaves no temporary file behind.
        store.save(&SECOND).unwrap();
        assert_eq!(store.load().unwrap(), Some(SECOND));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cursor_file_that_group_or_others_can_read_is_refused() {
        let dir = directory("broad");
        let path = dir.join("cursor");
        let mut store = FileCursorStore::open(&path).unwrap();
        store.save(&FIRST).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = store.load().unwrap_err();
        assert!(matches!(error, CursorFileError::Io(_)), "{error}");
        assert!(error.to_string().contains("0600"), "{error}");
        assert!(error.to_string().contains("cursor"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_does_not_hold_one_cursor_is_refused_as_malformed() {
        let dir = directory("corrupt");
        let path = dir.join("cursor");
        let store = FileCursorStore::open(&path).unwrap();
        let cases: [&[u8]; 5] = [
            b"",
            b"\n",
            b"000000000000010200000001\n\n",
            b"00000000000001020000000A\n",
            b"000000000000010200000001 trailing\n",
        ];
        for bytes in cases {
            write_owner_only(&path, bytes);
            let outcome = store.load();
            assert!(
                matches!(outcome, Err(CursorFileError::Malformed(_))),
                "{bytes:?}: {outcome:?}"
            );
        }
        // A refused file is left as it was: a load never rewrites it.
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"000000000000010200000001 trailing\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_that_others_can_write_to_is_refused_when_the_store_is_opened() {
        let dir = directory("open-dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let refused = FileCursorStore::open(&dir.join("cursor")).unwrap_err();
        assert!(refused.to_string().contains("group or others"), "{refused}");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_lock_on_the_same_cursor_file_is_refused_until_the_first_is_released() {
        let dir = directory("lock");
        let path = dir.join("cursor");
        let first = FileCursorStore::open(&path).unwrap();
        let held = first.lock().unwrap();
        let second = FileCursorStore::open(&path).unwrap();
        let refused = second.lock().unwrap_err();
        assert!(refused.to_string().contains("another watch"), "{refused}");
        drop(held);
        assert!(second.lock().is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
