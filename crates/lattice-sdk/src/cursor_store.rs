//! Saved feed cursors: the [`CursorStore`] trait, a strict text codec for one cursor, and an
//! in-memory store.
//!
//! A subscriber that restarts resumes from the last cursor it saved, so it neither misses nor
//! repeats events. This module does no I/O, reads no clock and draws no randomness. The
//! owner-only file store (atomic replace, mode 0600) belongs to the CLI layer and writes the bytes
//! that [`encode_cursor`] produces, so [`decode_cursor`] reads what that store writes.
//!
//! The stored form is the [`crate::mcp::cursor_to_hex`] spelling, 24 lowercase hex digits, followed
//! by one optional newline. [`decode_cursor`] refuses every other byte sequence, so one cursor has
//! at most two stored spellings and no input panics.

use std::fmt;

use crate::mcp::{FeedCursor, cursor_from_hex, cursor_to_hex};

/// A stored cursor that is not in the canonical form: wrong length, uppercase or non-hex digits,
/// or a byte after the optional newline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MalformedCursor;

impl fmt::Display for MalformedCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("stored cursor is malformed")
    }
}

impl std::error::Error for MalformedCursor {}

/// Encodes a cursor in its stored form: 24 lowercase hex digits and a newline.
#[must_use]
pub fn encode_cursor(cursor: &FeedCursor) -> String {
    let mut text = cursor_to_hex(cursor);
    text.push('\n');
    text
}

/// Decodes a stored cursor. The input is exactly 24 lowercase hex digits, optionally followed by
/// one newline. Any other byte, including a second newline, is refused.
pub fn decode_cursor(bytes: &[u8]) -> Result<FeedCursor, MalformedCursor> {
    let digits = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let text = std::str::from_utf8(digits).map_err(|_| MalformedCursor)?;
    cursor_from_hex(text).ok_or(MalformedCursor)
}

/// Saves the feed cursor a subscriber has reached, so a restart can resume from it.
pub trait CursorStore {
    /// The failure type. A stored value that does not decode reports [`MalformedCursor`].
    type Error: From<MalformedCursor>;

    /// Returns the saved cursor, or `None` when nothing has been saved yet.
    fn load(&self) -> Result<Option<FeedCursor>, Self::Error>;

    /// Replaces the saved cursor. A failed save must leave the previously saved cursor readable.
    fn save(&mut self, cursor: &FeedCursor) -> Result<(), Self::Error>;
}

/// An in-memory [`CursorStore`]. It keeps the stored bytes, so `load` decodes them the same way a
/// file store would.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryCursorStore {
    stored: Option<Vec<u8>>,
}

impl MemoryCursorStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that holds `bytes` exactly as given, for input that did not come from `save`.
    #[must_use]
    pub fn with_bytes(bytes: &[u8]) -> Self {
        Self {
            stored: Some(bytes.to_vec()),
        }
    }

    /// The stored bytes, or `None` when nothing has been saved.
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        self.stored.as_deref()
    }
}

impl CursorStore for MemoryCursorStore {
    type Error = MalformedCursor;

    fn load(&self) -> Result<Option<FeedCursor>, MalformedCursor> {
        match &self.stored {
            None => Ok(None),
            Some(bytes) => decode_cursor(bytes).map(Some),
        }
    }

    fn save(&mut self, cursor: &FeedCursor) -> Result<(), MalformedCursor> {
        self.stored = Some(encode_cursor(cursor).into_bytes());
        Ok(())
    }
}
