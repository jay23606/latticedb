//! Scheduler, work queue, drift engine, reconcilers, connector and pack hosts.
//!
//! Layer L5. Must not: Bypass core transactions.
#![forbid(unsafe_code)]

/// Layer marker used by the layer check. Do not remove.
pub const LAYER: u8 = 5;
