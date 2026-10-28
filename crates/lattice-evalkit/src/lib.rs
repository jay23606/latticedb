//! Eval harness: datasets with content hashes, providers under test, metrics with seeded bootstrap
//! confidence intervals, threshold gates, and reproducible reports.
//!
//! Layer dev. Must not: be a normal dependency of any non-dev crate.
//!
//! The harness tests itself: a deliberately bad provider must fail the gate and a perfect one must
//! pass it (see the tests in [`gate`] and [`drift`]), so a gate that can never fail cannot ship.
#![forbid(unsafe_code)]

pub mod dataset;
pub mod drift;
pub mod gate;
pub mod metrics;
pub mod report;
pub mod suite;

/// Trivial Phase 0 example metric kept for continuity: the fraction of `expected` found in `got`.
pub fn example_recall(expected: &[&str], got: &[&str]) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let hits = expected.iter().filter(|e| got.contains(e)).count();
    hits as f64 / expected.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_recall_counts_hits() {
        assert!((example_recall(&["a", "b"], &["a"]) - 0.5).abs() < f64::EPSILON);
        assert!((example_recall(&[], &[]) - 1.0).abs() < f64::EPSILON);
    }
}
