//! OIDC provider logic, tokens, keys, relationship store.
//!
//! Layer L4. Must not: Reach into engines directly.
#![forbid(unsafe_code)]

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 4;
