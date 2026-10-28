//! Threshold gates: turn metric results into a pass or fail that blocks a release.

use std::collections::BTreeMap;

/// A required lower bound for one metric.
#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    /// Metric name.
    pub name: String,
    /// The point estimate must reach this.
    pub lower_bound: f64,
    /// Confidence level of the interval (informational for the gate, used when computing it).
    pub confidence: f64,
    /// The interval's lower end may fall this far below `lower_bound`.
    pub tolerance: f64,
}

/// One metric's measured value and interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricResult {
    /// Point estimate.
    pub value: f64,
    /// Lower end of the confidence interval.
    pub ci_lo: f64,
    /// Upper end of the confidence interval.
    pub ci_hi: f64,
    /// How many examples the metric was computed from.
    pub n: usize,
}

/// One row of the gate verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Metric name.
    pub name: String,
    /// Measured value, if the metric was produced at all.
    pub result: Option<MetricResult>,
    /// The bound it was held to.
    pub bound: f64,
    /// Whether it passed.
    pub passed: bool,
    /// Why it failed.
    pub reason: String,
}

/// The verdict for a whole suite.
#[derive(Debug, Clone, PartialEq)]
pub struct GateReport {
    /// One row per threshold.
    pub rows: Vec<Row>,
    /// True only if every row passed.
    pub passed: bool,
}

/// Parses a `thresholds.toml` (`[[metric]]` tables).
pub fn parse_thresholds(text: &str) -> Result<Vec<Threshold>, String> {
    let v: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    let list = v
        .get("metric")
        .and_then(|m| m.as_array())
        .ok_or("no [[metric]] entries")?;
    let num = |m: &toml::Value, k: &str| -> Result<f64, String> {
        m.get(k)
            .and_then(|n| n.as_float().or_else(|| n.as_integer().map(|i| i as f64)))
            .ok_or_else(|| format!("metric needs a numeric `{k}`"))
    };
    let parsed: Vec<Threshold> = list
        .iter()
        .map(|m| {
            Ok(Threshold {
                name: m
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or("metric needs a `name`")?
                    .to_string(),
                lower_bound: num(m, "lower_bound")?,
                confidence: num(m, "confidence")?,
                tolerance: num(m, "tolerance")?,
            })
        })
        .collect::<Result<_, String>>()?;
    for t in &parsed {
        if !t.lower_bound.is_finite() || !t.tolerance.is_finite() || t.tolerance < 0.0 {
            return Err(format!(
                "metric `{}`: lower_bound and tolerance must be finite and tolerance >= 0",
                t.name
            ));
        }
        if !(t.confidence > 0.0 && t.confidence < 1.0) {
            return Err(format!("metric `{}`: confidence must be in (0, 1)", t.name));
        }
    }
    Ok(parsed)
}

/// Holds results to thresholds. A metric that is missing from the results fails.
pub fn evaluate(thresholds: &[Threshold], results: &BTreeMap<String, MetricResult>) -> GateReport {
    let rows: Vec<Row> = thresholds
        .iter()
        .map(|t| match results.get(&t.name) {
            None => Row {
                name: t.name.clone(),
                result: None,
                bound: t.lower_bound,
                passed: false,
                reason: "metric was not produced".into(),
            },
            Some(r) if !(r.value.is_finite() && r.ci_lo.is_finite() && r.ci_hi.is_finite()) => {
                Row {
                    name: t.name.clone(),
                    result: Some(*r),
                    bound: t.lower_bound,
                    passed: false,
                    reason: "result is not a finite number".into(),
                }
            }
            Some(r) if r.ci_lo > r.value || r.value > r.ci_hi || r.ci_lo > r.ci_hi => Row {
                name: t.name.clone(),
                result: Some(*r),
                bound: t.lower_bound,
                passed: false,
                reason: "interval does not contain the value".into(),
            },
            Some(r) if r.n == 0 => Row {
                name: t.name.clone(),
                result: Some(*r),
                bound: t.lower_bound,
                passed: false,
                reason: "metric was computed from zero examples".into(),
            },
            Some(r) => {
                let value_ok = r.value >= t.lower_bound;
                let ci_ok = r.ci_lo >= t.lower_bound - t.tolerance;
                let reason = match (value_ok, ci_ok) {
                    (true, true) => String::new(),
                    (false, _) => format!("value {:.4} is below {:.4}", r.value, t.lower_bound),
                    (true, false) => format!(
                        "interval lower end {:.4} is below {:.4} minus tolerance {:.4}",
                        r.ci_lo, t.lower_bound, t.tolerance
                    ),
                };
                Row {
                    name: t.name.clone(),
                    result: Some(*r),
                    bound: t.lower_bound,
                    passed: value_ok && ci_ok,
                    reason,
                }
            }
        })
        .collect();
    let passed = !rows.is_empty() && rows.iter().all(|r| r.passed);
    GateReport { rows, passed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(v: f64, lo: f64) -> MetricResult {
        MetricResult {
            value: v,
            ci_lo: lo,
            ci_hi: 1.0,
            n: 100,
        }
    }

    const TOML: &str =
        "[[metric]]\nname=\"f1\"\nlower_bound=0.8\nconfidence=0.95\ntolerance=0.02\n";

    #[test]
    fn passes_fails_and_missing_metrics() {
        let t = parse_thresholds(TOML).unwrap();
        let mut ok = BTreeMap::new();
        ok.insert("f1".to_string(), res(0.9, 0.85));
        assert!(evaluate(&t, &ok).passed);
        let mut low = BTreeMap::new();
        low.insert("f1".to_string(), res(0.7, 0.6));
        assert!(!evaluate(&t, &low).passed);
        let mut wide = BTreeMap::new();
        wide.insert("f1".to_string(), res(0.85, 0.5));
        assert!(
            !evaluate(&t, &wide).passed,
            "a wide interval must fail even when the point estimate passes"
        );
        assert!(!evaluate(&t, &BTreeMap::new()).passed);
        assert!(
            !evaluate(&[], &ok).passed,
            "an empty gate must not pass vacuously"
        );
    }

    #[test]
    fn invalid_evidence_and_configuration_cannot_pass() {
        let t = parse_thresholds(TOML).unwrap();
        let mut r = BTreeMap::new();
        r.insert(
            "f1".to_string(),
            MetricResult {
                value: f64::INFINITY,
                ci_lo: f64::INFINITY,
                ci_hi: f64::NAN,
                n: 10,
            },
        );
        assert!(!evaluate(&t, &r).passed, "non-finite results must fail");
        r.insert(
            "f1".to_string(),
            MetricResult {
                value: 0.95,
                ci_lo: 0.99,
                ci_hi: 1.0,
                n: 10,
            },
        );
        assert!(
            !evaluate(&t, &r).passed,
            "an interval that excludes the value is invalid"
        );
        r.insert(
            "f1".to_string(),
            MetricResult {
                value: 1.0,
                ci_lo: 1.0,
                ci_hi: 1.0,
                n: 0,
            },
        );
        assert!(
            !evaluate(&t, &r).passed,
            "a metric from zero examples must fail"
        );
        assert!(
            parse_thresholds(
                "[[metric]]\nname=\"f\"\nlower_bound=0.5\nconfidence=1.5\ntolerance=0.0\n"
            )
            .is_err()
        );
        assert!(
            parse_thresholds(
                "[[metric]]\nname=\"f\"\nlower_bound=0.5\nconfidence=0.9\ntolerance=-1.0\n"
            )
            .is_err()
        );
        assert!(
            parse_thresholds(
                "[[metric]]\nname=\"f\"\nlower_bound=nan\nconfidence=0.9\ntolerance=0.0\n"
            )
            .is_err()
        );
    }

    #[test]
    fn bad_threshold_files_are_rejected() {
        assert!(parse_thresholds("x=1").is_err());
        assert!(parse_thresholds("[[metric]]\nname=\"f1\"\n").is_err());
    }
}
