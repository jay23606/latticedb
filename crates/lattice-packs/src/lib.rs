//! Pack runtime and bundled pack sources.
//!
//! Layer L5. Must not: Edit the kernel.
#![forbid(unsafe_code)]

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 5;
