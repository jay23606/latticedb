//! Stable ids, error taxonomy, hybrid logical clock timestamps and on-disk enums.
//!
//! Layer L0. Must not: depend on any other lattice crate or do I/O.
#![forbid(unsafe_code)]

pub mod error;
pub mod hlc;
pub mod id;
pub mod secret;

pub use error::{Classify, ErrorClass};
pub use hlc::Hlc;
pub use id::{Id, IdParseError};
pub use secret::Secret;

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 0;
