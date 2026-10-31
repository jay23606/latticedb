//! Reconnect backoff: how long a client waits before its next attempt, with a ceiling and jitter.
//!
//! [`Backoff`] is a pure state machine in the style of [`crate::outbox::Outbox`]: no I/O, no clock
//! and no randomness. The caller owns the clock, and it supplies the jitter as a uniform sample over
//! `u64` from a source it trusts to be unpredictable enough to spread clients out. The ceiling for
//! attempt `n` is `min(cap_ms, base_ms * 2^n)`, and the delay is uniform over `0..=ceiling` (the
//! "full jitter" scheme), so agents that lost the same server do not reconnect in lockstep.
//!
//! A delay can be zero: a jitter of 0 gives 0, and full jitter includes that case. This type adds
//! no minimum. A driver that wants a minimum wait before each reconnect must enforce it itself.
//!
//! The ceiling never shrinks and never exceeds `cap_ms`, for any number of attempts. A delay never
//! exceeds its ceiling, and a larger jitter never gives a shorter delay. [`Backoff::reset`] returns
//! to the first attempt; the driver calls it once a frame arrives on a healthy connection.

use std::fmt;

/// The configuration is refused: `base_ms` must be at least 1 and `cap_ms` at least `base_ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBackoff;

impl fmt::Display for InvalidBackoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("backoff needs 1 <= base_ms <= cap_ms")
    }
}

impl std::error::Error for InvalidBackoff {}

/// Exponential reconnect delays with full jitter and a ceiling. See the module documentation. A
/// delay may be zero (jitter 0), so a caller that needs a minimum wait enforces it itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backoff {
    base_ms: u64,
    cap_ms: u64,
    attempt: u32,
}

impl Backoff {
    /// A backoff whose first ceiling is `base_ms` and whose delays never exceed `cap_ms`.
    pub fn new(base_ms: u64, cap_ms: u64) -> Result<Self, InvalidBackoff> {
        if base_ms == 0 || cap_ms < base_ms {
            return Err(InvalidBackoff);
        }
        Ok(Self {
            base_ms,
            cap_ms,
            attempt: 0,
        })
    }

    /// The delay before the next attempt, in milliseconds, and moves on to the following attempt.
    /// `jitter` is a uniform sample over `u64`: the delay is `(ceiling + 1) * jitter / 2^64`, so it
    /// lies in `0..=ceiling`, is 0 for a jitter of 0 and is the ceiling for `u64::MAX`. Zero is a
    /// possible delay, not a bug: enforce a minimum in the driver if one is wanted.
    #[must_use]
    pub fn next_delay(&mut self, jitter: u64) -> u64 {
        let ceiling = self.ceiling();
        self.attempt = self.attempt.saturating_add(1);
        scale(ceiling, jitter)
    }

    /// Returns to the first attempt. Call it after a frame arrives on a healthy connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    fn ceiling(&self) -> u64 {
        // Past the width of u64 the factor saturates, and so does the product: only the cap is left.
        let factor = 1u64.checked_shl(self.attempt).unwrap_or(u64::MAX);
        self.base_ms.saturating_mul(factor).min(self.cap_ms)
    }
}

/// `ceiling` scaled by `jitter / 2^64`, rounded down, in `0..=ceiling`. The product is computed in
/// `u128` so it cannot overflow, and `(ceiling + 1) * jitter >> 64` is at most `ceiling`.
fn scale(ceiling: u64, jitter: u64) -> u64 {
    let product = (u128::from(ceiling) + 1) * u128::from(jitter);
    u64::try_from(product >> 64).unwrap_or(ceiling)
}

#[cfg(test)]
mod tests {
    use super::{Backoff, scale};

    #[test]
    fn saturated_attempt_counter_keeps_the_cap() {
        let mut b = Backoff {
            base_ms: 1,
            cap_ms: 1_000,
            attempt: u32::MAX,
        };
        assert_eq!(b.next_delay(u64::MAX), 1_000);
        assert_eq!(b.attempt, u32::MAX);
    }

    #[test]
    fn scale_maps_the_full_range_onto_zero_to_the_ceiling() {
        assert_eq!(scale(0, u64::MAX), 0);
        assert_eq!(scale(1, u64::MAX), 1);
        assert_eq!(scale(u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(scale(u64::MAX, 0), 0);
    }
}
