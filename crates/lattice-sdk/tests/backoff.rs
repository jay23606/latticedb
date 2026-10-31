//! Backoff tests: the ceiling and its cap, the jitter mapping, reset and the refused configurations.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use lattice_sdk::backoff::{Backoff, InvalidBackoff};
use proptest::prelude::*;

#[test]
fn full_jitter_ceiling_doubles_from_the_base_and_stops_at_the_cap() {
    let mut b = Backoff::new(100, 1_000).unwrap();
    let ceilings: Vec<u64> = (0..8).map(|_| b.next_delay(u64::MAX)).collect();
    assert_eq!(ceilings, [100, 200, 400, 800, 1_000, 1_000, 1_000, 1_000]);
}

#[test]
fn jitter_maps_linearly_onto_the_window() {
    let mut zero = Backoff::new(100, 1_000).unwrap();
    assert_eq!(zero.next_delay(0), 0);
    // (100 + 1) * 2^63 / 2^64 = 50.5, rounded down.
    let mut half = Backoff::new(100, 1_000).unwrap();
    assert_eq!(half.next_delay(1 << 63), 50);
    let mut full = Backoff::new(100, 1_000).unwrap();
    assert_eq!(full.next_delay(u64::MAX), 100);
}

#[test]
fn the_ceiling_keeps_growing_to_a_very_large_cap_without_overflow() {
    let mut b = Backoff::new(1, u64::MAX).unwrap();
    let delays: Vec<u64> = (0..70).map(|_| b.next_delay(u64::MAX)).collect();
    for (k, d) in delays.iter().take(64).enumerate() {
        assert_eq!(*d, 1u64 << k, "attempt {k}");
    }
    assert_eq!(delays[64], u64::MAX);
    assert_eq!(delays[69], u64::MAX);
}

#[test]
fn a_ceiling_whose_doubling_overflows_saturates_at_the_cap() {
    // 3 * 2^62 still fits in u64; 3 * 2^63 does not, so the checked multiply must give the cap.
    let mut b = Backoff::new(3, u64::MAX).unwrap();
    let delays: Vec<u64> = (0..70).map(|_| b.next_delay(u64::MAX)).collect();
    for (k, d) in delays.iter().take(63).enumerate() {
        assert_eq!(*d, 3u64 << k, "attempt {k}");
    }
    assert_eq!(delays[63], u64::MAX);
    assert_eq!(delays[69], u64::MAX);
}

#[test]
fn reset_returns_to_the_first_window() {
    let mut b = Backoff::new(100, 60_000).unwrap();
    for _ in 0..5 {
        let _ = b.next_delay(u64::MAX);
    }
    b.reset();
    assert_eq!(b.next_delay(u64::MAX), 100);
    assert_eq!(b.next_delay(u64::MAX), 200);
}

#[test]
fn refuses_a_zero_base_and_a_cap_below_the_base() {
    assert_eq!(Backoff::new(0, 10), Err(InvalidBackoff));
    assert_eq!(Backoff::new(10, 9), Err(InvalidBackoff));
    assert!(Backoff::new(10, 10).is_ok());
    assert!(Backoff::new(1, u64::MAX).is_ok());
}

proptest! {
    #[test]
    fn every_delay_stays_within_its_ceiling_and_the_cap(
        base in 1u64..=10_000,
        extra in 0u64..=1_000_000,
        jitters in proptest::collection::vec(any::<u64>(), 1..120),
    ) {
        let cap = base + extra;
        let mut top = Backoff::new(base, cap).unwrap();
        let ceilings: Vec<u64> = jitters.iter().map(|_| top.next_delay(u64::MAX)).collect();
        prop_assert_eq!(ceilings[0], base);
        prop_assert!(ceilings.windows(2).all(|w| w[0] <= w[1]));
        prop_assert!(ceilings.iter().all(|c| *c <= cap));

        let mut b = Backoff::new(base, cap).unwrap();
        for (i, jitter) in jitters.iter().enumerate() {
            let delay = b.next_delay(*jitter);
            prop_assert!(delay <= ceilings[i]);
            prop_assert!(delay <= cap);
        }
    }

    #[test]
    fn a_larger_jitter_never_waits_less(
        base in 1u64..=10_000,
        extra in 0u64..=1_000_000,
        attempts in 0usize..80,
        a in any::<u64>(),
        b in any::<u64>(),
    ) {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut first = Backoff::new(base, base + extra).unwrap();
        for _ in 0..attempts {
            let _ = first.next_delay(0);
        }
        let mut second = first.clone();
        prop_assert!(first.next_delay(lo) <= second.next_delay(hi));
    }

    #[test]
    fn the_same_inputs_give_the_same_delays(
        base in 1u64..=10_000,
        extra in 0u64..=1_000_000,
        jitters in proptest::collection::vec(any::<u64>(), 0..60),
    ) {
        let mut one = Backoff::new(base, base + extra).unwrap();
        let mut two = Backoff::new(base, base + extra).unwrap();
        let first: Vec<u64> = jitters.iter().map(|j| one.next_delay(*j)).collect();
        let second: Vec<u64> = jitters.iter().map(|j| two.next_delay(*j)).collect();
        prop_assert_eq!(first, second);
    }
}
