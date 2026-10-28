//! The only crate allowed to contain unsafe code (io_uring, mmap wrappers).
//!
//! Layer L0. Must not: Contain business logic, or be used except through lattice-seams.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 0;
