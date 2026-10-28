//! Optional models plugin (feature intel, off by default).
//!
//! Layer L5. Must not: Be required by any other crate.
#![forbid(unsafe_code)]

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 5;
