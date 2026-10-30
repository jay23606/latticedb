//! Numeric arithmetic, rounding, comparison and casts, with the expected texts taken from a run of the
//! same statement on PostgreSQL 17. Two expectations are the executor's own limits, not PostgreSQL's, and
//! are marked as such where they appear: the 38-fractional-digit limit (the literal test at the end) and
//! the 22003 error for a 41-digit integer sum. The division scale rule is pinned in `numeric_division.rs`;
//! the divisions here are a few quotients that PostgreSQL 17 prints with 20 fractional digits.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{admin, err, fixture, rows};

/// (statement, the text PostgreSQL 17 prints for the single value it returns)
#[rustfmt::skip]
const CASES: [(&str, &str); 75] = [
    ("SELECT 1.5 + 2.25", "3.75"),
    ("SELECT 0.1 + 0.2", "0.3"),
    ("SELECT 2.5 * 4", "10.0"),
    ("SELECT 5 - 7.5", "-2.5"),
    ("SELECT -0.0", "0.0"),
    ("SELECT .5 + 1", "1.5"),
    ("SELECT 5. + 1", "6"),
    ("SELECT 1e3", "1000"),
    ("SELECT 1.5e-2 + 0", "0.015"),
    ("SELECT 1E+2", "100"),
    ("SELECT 12.50", "12.50"),
    ("SELECT 0.000", "0.000"),
    ("SELECT 007.5", "7.5"),
    ("SELECT round(2.5)", "3"),
    ("SELECT round(-2.5)", "-3"),
    ("SELECT round(0.5)", "1"),
    ("SELECT round(-0.5)", "-1"),
    ("SELECT round(2.345, 2)", "2.35"),
    ("SELECT round(2.344, 2)", "2.34"),
    ("SELECT round(-2.345, 2)", "-2.35"),
    ("SELECT round(2.5, 0)", "3"),
    ("SELECT round(1.995, 2)", "2.00"),
    ("SELECT round(9.995, 2)", "10.00"),
    ("SELECT round(0.004, 2)", "0.00"),
    ("SELECT round(-0.004, 2)", "0.00"),
    ("SELECT abs(-3.25)", "3.25"),
    ("SELECT abs(3.25)", "3.25"),
    ("SELECT abs(-0.0)", "0.0"),
    ("SELECT 1.0 = 1.00", "t"),
    ("SELECT 1.5 < 2", "t"),
    ("SELECT 2.5 > 2.5", "f"),
    ("SELECT -1.5 < -1.25", "t"),
    ("SELECT -1.25 < -1.5", "f"),
    ("SELECT 0.30 > 0.3", "f"),
    ("SELECT 1.5 <= 1.5", "t"),
    ("SELECT 2 >= 2.5", "f"),
    ("SELECT 10 > 9.99", "t"),
    ("SELECT -10 < -9.99", "t"),
    ("SELECT 0.0 = -0.0", "t"),
    ("SELECT 1.5::int", "2"),
    ("SELECT 2.5::int", "3"),
    ("SELECT (-1.5)::int", "-2"),
    ("SELECT (-2.5)::int", "-3"),
    ("SELECT 0.5::int", "1"),
    ("SELECT 100000000000000000000 + 1", "100000000000000000001"),
    ("SELECT 123456789012345678901234567890.5 * 2", "246913578024691357802469135781.0"),
    ("SELECT 99999999999999999999999999999999999999 + 1", "100000000000000000000000000000000000000"),
    ("SELECT 1.5 * -2", "-3.0"),
    ("SELECT -1.5 * -2", "3.0"),
    ("SELECT 0.1::double precision::numeric", "0.1"),
    ("SELECT 2.5::double precision::numeric", "2.5"),
    ("SELECT 0::double precision::numeric", "0"),
    ("SELECT (-1.25)::double precision::numeric", "-1.25"),
    ("SELECT 5 % 3.0", "2.0"),
    ("SELECT -5 % 3.0", "-2.0"),
    ("SELECT 5.5 % 2", "1.5"),
    ("SELECT 1.0 / 3.0", "0.33333333333333333333"),
    ("SELECT 2.0 / 3.0", "0.66666666666666666667"),
    ("SELECT -2.0 / 3.0", "-0.66666666666666666667"),
    ("SELECT 2.0 / -3.0", "-0.66666666666666666667"),
    ("SELECT -2.0 / -3.0", "0.66666666666666666667"),
    ("SELECT 1.0 / 6.0", "0.16666666666666666667"),
    ("SELECT 5.0 / 6.0", "0.83333333333333333333"),
    ("SELECT 7.0 / 9.0", "0.77777777777777777778"),
    ("SELECT 1.0 / 8.0", "0.12500000000000000000"),
    ("SELECT 0.5 / 3", "0.16666666666666666667"),
    ("SELECT 100000000000000000000000000000000000 > 0.000000000000000000000000000000000001", "t"),
    ("SELECT 0.000000000000000000000000000000000001 < 100000000000000000000000000000000000", "t"),
    ("SELECT -100000000000000000000000000000000000 < 0.000000000000000000000000000000000001", "t"),
    ("SELECT 0.000000000000000000000000000000000001 > -100000000000000000000000000000000000", "t"),
    ("SELECT 100000000000000000000000000000000000 = 0.000000000000000000000000000000000001", "f"),
    ("SELECT 1.5 = 1.6", "f"),
    ("SELECT 1.5 <> 1.5", "f"),
    ("SELECT 1.5 <> 1.6", "t"),
    ("SELECT 0.0 = 0.00", "t"),
];

#[test]
fn numeric_results_print_exactly_as_postgresql_prints_them() {
    let mut f = fixture();
    let mut s = admin();
    for (sql, expected) in CASES {
        assert_eq!(
            rows(&mut f.db, &mut s, sql),
            vec![vec![expected.to_string()]],
            "{sql}"
        );
    }
}

#[test]
fn division_by_a_numeric_zero_is_22012_and_a_value_beyond_38_digits_is_22003() {
    let mut f = fixture();
    let mut s = admin();
    assert_eq!(err(&mut f.db, &mut s, "SELECT 1 / 0.0").sqlstate(), "22012");
    assert_eq!(err(&mut f.db, &mut s, "SELECT 5 % 0.0").sqlstate(), "22012");
    // PostgreSQL 17 returns this sum; 22003 is the executor's own range limit.
    assert_eq!(
        err(
            &mut f.db,
            &mut s,
            "SELECT 12345678901234567890123456789012345678901 + 1"
        )
        .sqlstate(),
        "22003"
    );
}

#[test]
fn a_literal_with_more_than_38_fraction_digits_is_rounded_half_away_from_zero_at_38() {
    // PostgreSQL keeps every digit; this executor stores at most 38 and says so (documented limit).
    let mut f = fixture();
    let mut s = admin();
    let nines = "9".repeat(39);
    assert_eq!(
        rows(&mut f.db, &mut s, &format!("SELECT 0.{nines}")),
        vec![vec![format!("1.{}", "0".repeat(38))]]
    );
    let fours = "4".repeat(39);
    assert_eq!(
        rows(&mut f.db, &mut s, &format!("SELECT 0.{fours}")),
        vec![vec![format!("0.{}", "4".repeat(38))]]
    );
    let five = format!("{}5", "0".repeat(38));
    assert_eq!(
        rows(&mut f.db, &mut s, &format!("SELECT 0.{five}")),
        vec![vec![format!("0.{}1", "0".repeat(37))]]
    );
    // Only the first dropped digit decides: 4 is below half a unit in the 38th place, so a 5 after it
    // must not round up.
    let zeros = "0".repeat(38);
    assert_eq!(
        rows(&mut f.db, &mut s, &format!("SELECT 0.{zeros}45")),
        vec![vec![format!("0.{zeros}")]]
    );
}
