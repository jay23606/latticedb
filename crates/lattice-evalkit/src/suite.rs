//! Running a provider over a dataset.

use crate::dataset::Dataset;
use crate::gate::MetricResult;
use crate::metrics::{bootstrap_micro_prf, prf};
use std::collections::BTreeMap;

/// The thing under evaluation: a rule, an agent, or a model. Phase 1 uses deterministic stand-ins.
pub trait Provider {
    /// Name for reports.
    fn name(&self) -> &str;
    /// Returns the labels it predicts for `input`.
    fn predict(&self, input: &str) -> Vec<String>;
}

/// Runs `provider` over `dataset` and returns precision, recall and f1. Each interval resamples
/// examples and recomputes the same micro-averaged metric, so it estimates the reported statistic.
/// `confidence` sets the interval; `seed` makes it reproducible.
pub fn run(
    provider: &dyn Provider,
    dataset: &Dataset,
    confidence: f64,
    seed: u64,
) -> BTreeMap<String, MetricResult> {
    let pairs: Vec<(Vec<String>, Vec<String>)> = dataset
        .examples
        .iter()
        .map(|e| (e.expected.clone(), provider.predict(&e.input)))
        .collect();
    let overall = prf(&pairs);
    let [p, r, f] = bootstrap_micro_prf(&pairs, 400, seed, confidence);
    let n = pairs.len();
    let mut out = BTreeMap::new();
    for (name, value, (ci_lo, ci_hi)) in [
        ("precision", overall.precision, p),
        ("recall", overall.recall, r),
        ("f1", overall.f1, f),
    ] {
        out.insert(
            name.to_string(),
            MetricResult {
                value,
                ci_lo,
                ci_hi,
                n,
            },
        );
    }
    out
}
