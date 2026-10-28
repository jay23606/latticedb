//! Reports. The output is a pure function of the inputs, so identical runs give identical bytes.

use crate::gate::GateReport;

/// A JSON number, or `null` for a value JSON cannot represent.
fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.6}")
    } else {
        "null".to_string()
    }
}

/// Escapes a string for use inside a JSON string literal, including control characters.
fn esc_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Escapes text for HTML element content and attribute values.
fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Deterministic JSON for a gate verdict.
pub fn to_json(suite: &str, dataset_hash: &str, provider: &str, report: &GateReport) -> String {
    let rows: Vec<String> = report
        .rows
        .iter()
        .map(|r| {
            let (v, lo, hi, n) = r.result.map_or(
                ("null".to_string(), "null".to_string(), "null".to_string(), "0".to_string()),
                |m| (num(m.value), num(m.ci_lo), num(m.ci_hi), m.n.to_string()),
            );
            format!(
                "{{\"metric\":\"{}\",\"value\":{v},\"ci_lo\":{lo},\"ci_hi\":{hi},\"n\":{n},\"bound\":{},\"passed\":{},\"reason\":\"{}\"}}",
                esc_json(&r.name),
                num(r.bound),
                r.passed,
                esc_json(&r.reason)
            )
        })
        .collect();
    format!(
        "{{\"suite\":\"{}\",\"dataset_hash\":\"{}\",\"provider\":\"{}\",\"passed\":{},\"metrics\":[{}]}}",
        esc_json(suite),
        esc_json(dataset_hash),
        esc_json(provider),
        report.passed,
        rows.join(",")
    )
}

/// A small HTML table for people. Every inserted string is escaped.
pub fn to_html(suite: &str, provider: &str, report: &GateReport) -> String {
    let mut out = format!(
        "<h1>{}: {}</h1><p>provider: {}</p><table><tr><th>metric</th><th>value</th><th>interval</th><th>bound</th><th>result</th></tr>",
        esc_html(suite),
        if report.passed { "PASS" } else { "FAIL" },
        esc_html(provider)
    );
    for r in &report.rows {
        let (v, iv) = r.result.map_or(("-".into(), "-".into()), |m| {
            (
                format!("{:.4}", m.value),
                format!("[{:.4}, {:.4}]", m.ci_lo, m.ci_hi),
            )
        });
        out.push_str(&format!(
            "<tr><td>{}</td><td>{v}</td><td>{iv}</td><td>{:.4}</td><td>{}</td></tr>",
            esc_html(&r.name),
            r.bound,
            if r.passed {
                "pass".to_string()
            } else {
                esc_html(&r.reason)
            }
        ));
    }
    out.push_str("</table>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::{MetricResult, Row};

    fn report(name: &str, reason: &str, value: f64) -> GateReport {
        GateReport {
            rows: vec![Row {
                name: name.into(),
                result: Some(MetricResult {
                    value,
                    ci_lo: 0.0,
                    ci_hi: 1.0,
                    n: 3,
                }),
                bound: 0.5,
                passed: false,
                reason: reason.into(),
            }],
            passed: false,
        }
    }

    #[test]
    fn json_escapes_control_characters_and_nulls_non_finite_numbers() {
        let j = to_json(
            "s\u{1}",
            "h",
            "p\tq",
            &report("m\"x", "line\nbreak\r", f64::NAN),
        );
        assert!(!j.contains('\u{1}') && !j.contains('\t') && !j.contains('\r'));
        assert!(j.contains("\\u0001") && j.contains("\\t") && j.contains("\"value\":null"));
    }

    #[test]
    fn html_neutralizes_markup_from_provider_suite_and_metric_names() {
        let h = to_html(
            "<script>alert(1)</script>",
            "<img onerror=x>",
            &report("<b>", "<i>", 0.1),
        );
        assert!(
            !h.contains("<script>")
                && !h.contains("<img")
                && !h.contains("<b>")
                && !h.contains("<i>")
        );
        assert!(h.contains("&lt;script&gt;"));
    }
}
