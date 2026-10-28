//! Simulated wall clock.

use lattice_seams::Clock;
use std::cell::Cell;

/// A clock the scenario advances by hand. It may also be set backwards to model clock skew.
#[derive(Debug, Default)]
pub struct SimClock {
    now_ms: Cell<u64>,
}

impl SimClock {
    /// Starts at `start_ms`.
    pub fn new(start_ms: u64) -> Self {
        Self {
            now_ms: Cell::new(start_ms),
        }
    }

    /// Moves time forward.
    pub fn advance(&self, ms: u64) {
        self.now_ms.set(self.now_ms.get().saturating_add(ms));
    }

    /// Sets time to an arbitrary value, including a smaller one.
    pub fn set(&self, ms: u64) {
        self.now_ms.set(ms);
    }
}

impl Clock for SimClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_and_set() {
        let c = SimClock::new(10);
        c.advance(5);
        assert_eq!(c.now_ms(), 15);
        c.set(3);
        assert_eq!(c.now_ms(), 3);
    }
}
