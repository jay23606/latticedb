//! Mutation-based ground truth for drift: take a consistent snapshot, change known facts, and the
//! expected drifts are known by construction. Includes deterministic stand-in providers used to
//! prove the harness itself works.

use crate::dataset::{Dataset, Example, Split};
use crate::suite::Provider;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Builds `facts` claim-versus-observation examples; about `mutation_percent` of them are mutated
/// so the observed value differs from the claim. The expected label is `drift:<key>`.
pub fn generate(seed: u64, facts: usize, mutation_percent: u64) -> Dataset {
    let mut rng = Rng(seed);
    let examples = (0..facts)
        .map(|i| {
            let key = format!("setting_{i}");
            let claimed = 10 + rng.below(990);
            let mutated = rng.below(100) < mutation_percent;
            let observed = if mutated {
                claimed + 1 + rng.below(50)
            } else {
                claimed
            };
            Example {
                input: format!("claim: {key} = {claimed}; observed: {key} = {observed}"),
                expected: if mutated {
                    vec![format!("drift:{key}")]
                } else {
                    Vec::new()
                },
            }
        })
        .collect();
    Dataset::new(
        "drift-injection",
        1,
        Split::Dev,
        "synthetic, mutation-based",
        examples,
    )
}

fn parse(input: &str) -> Option<(String, u64, u64)> {
    let (claim, observed) = input.split_once("; observed: ")?;
    let (key, c) = claim.strip_prefix("claim: ")?.split_once(" = ")?;
    let (_, o) = observed.split_once(" = ")?;
    Some((
        key.to_string(),
        c.trim().parse().ok()?,
        o.trim().parse().ok()?,
    ))
}

/// A deterministic comparator: reports drift exactly when the numbers differ.
pub struct ExactComparator;

impl Provider for ExactComparator {
    fn name(&self) -> &str {
        "exact-comparator"
    }
    fn predict(&self, input: &str) -> Vec<String> {
        match parse(input) {
            Some((k, c, o)) if c != o => vec![format!("drift:{k}")],
            _ => Vec::new(),
        }
    }
}

/// Reports nothing: zero recall.
pub struct Silent;

impl Provider for Silent {
    fn name(&self) -> &str {
        "silent"
    }
    fn predict(&self, _input: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Reports drift for everything: low precision.
pub struct Noisy;

impl Provider for Noisy {
    fn name(&self) -> &str {
        "noisy"
    }
    fn predict(&self, input: &str) -> Vec<String> {
        parse(input)
            .map(|(k, _, _)| vec![format!("drift:{k}")])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::{evaluate, parse_thresholds};
    use crate::{report, suite};

    const THRESHOLDS: &str = "[[metric]]\nname=\"f1\"\nlower_bound=0.9\nconfidence=0.95\ntolerance=0.02\n\n[[metric]]\nname=\"precision\"\nlower_bound=0.9\nconfidence=0.95\ntolerance=0.02\n";

    #[test]
    fn generation_is_reproducible_and_ground_truth_matches_mutations() {
        let a = generate(7, 200, 30);
        assert_eq!(a.hash, generate(7, 200, 30).hash);
        assert_ne!(a.hash, generate(8, 200, 30).hash);
        let drifts = a.examples.iter().filter(|e| !e.expected.is_empty()).count();
        assert!(
            (30..90).contains(&drifts),
            "roughly 30 percent mutated: {drifts}"
        );
    }

    #[test]
    fn harness_self_test_perfect_passes_and_bad_providers_fail() {
        let data = generate(1, 300, 30);
        let t = parse_thresholds(THRESHOLDS).unwrap();
        let good = evaluate(&t, &suite::run(&ExactComparator, &data, 0.95, 5));
        assert!(good.passed, "{good:?}");
        let silent = evaluate(&t, &suite::run(&Silent, &data, 0.95, 5));
        assert!(
            !silent.passed,
            "a provider that finds nothing must fail the gate"
        );
        let noisy = evaluate(&t, &suite::run(&Noisy, &data, 0.95, 5));
        assert!(
            !noisy.passed,
            "a provider that flags everything must fail the gate"
        );
    }

    #[test]
    fn reports_are_byte_identical_for_identical_inputs() {
        let data = generate(3, 100, 30);
        let t = parse_thresholds(THRESHOLDS).unwrap();
        let make = || {
            let r = evaluate(&t, &suite::run(&ExactComparator, &data, 0.95, 9));
            report::to_json("drift", &data.hash, "exact-comparator", &r)
        };
        assert_eq!(make(), make());
        let other_seed = {
            let r = evaluate(&t, &suite::run(&ExactComparator, &data, 0.95, 10));
            report::to_json("drift", &data.hash, "exact-comparator", &r)
        };
        // Different bootstrap seed changes the interval digits but not the verdict.
        assert!(other_seed.contains("\"passed\":true"));
    }

    #[test]
    fn missing_a_metric_fails_the_gate() {
        let data = generate(1, 50, 30);
        let t = parse_thresholds(
            "[[metric]]\nname=\"nonexistent\"\nlower_bound=0.1\nconfidence=0.95\ntolerance=0.0\n",
        )
        .unwrap();
        assert!(!evaluate(&t, &suite::run(&ExactComparator, &data, 0.95, 5)).passed);
    }
}
