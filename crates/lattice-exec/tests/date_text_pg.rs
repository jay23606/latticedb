//! Text after the date in a `date` or `timestamptz` input, checked against PostgreSQL 17 at UTC: the
//! time of day is read and ignored, an invalid time or stray text is refused, and `AD` is accepted.
//! `BC` years are accepted by PostgreSQL but are outside this server's date range, so they are
//! refused instead of being read as the AD date in front of them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{err, fixture, rows};

const CASES: &[(&str, &[&str])] = &[
    ("SELECT '2024-03-15 AD'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 junk'::date", &["ERR 22007"]),
    ("SELECT '2024-03-15 10:00:00'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15T10:00:00'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 25:00:00'::date", &["ERR 22008"]),
    ("SELECT '2024-03-15 10:00:00+02'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 10:00:00 junk'::date", &["ERR 22007"]),
    ("SELECT '2024-03-15 ad'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 aD'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 10:00:00 AD'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 10:00:00 ad'::date", &["2024-03-15"]),
    ("SELECT '2024-03-15 10:00:00Zad'::date", &["ERR 22007"]),
    ("SELECT '2024-03-15 25:00:00 AD'::date", &["ERR 22008"]),
    (
        "SELECT '2024-03-15 10:00:00+00 junk'::timestamptz",
        &["ERR 22007"],
    ),
    (
        "SELECT '2024-03-15 10:00:00+00 '::timestamptz",
        &["2024-03-15 10:00:00+00"],
    ),
    ("SELECT ' 2024-03-15 '::date", &["2024-03-15"]),
    ("SELECT '2024-03-15'::date", &["2024-03-15"]),
    ("SELECT '0000-01-01'::date", &["ERR 22008"]),
];

#[test]
fn text_after_a_date_is_read_as_postgresql_reads_it() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in CASES {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) => common::text(&r).iter().map(|row| row.join("|")).collect(),
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}

#[test]
fn a_bc_year_is_refused_and_never_read_as_ad() {
    let mut f = fixture();
    let mut s = common::admin();
    for text in [
        "2024-03-15 BC",
        "2024-03-15 bc",
        "2024-03-15 10:00:00 BC",
        "0001-01-01 BC",
        "4713-01-01 BC",
    ] {
        let sql = format!("SELECT '{text}'::date");
        assert_eq!(err(&mut f.db, &mut s, &sql).sqlstate(), "22007", "{sql}");
    }
    assert_eq!(
        rows(&mut f.db, &mut s, "SELECT '2024-03-15 AD'::date"),
        [["2024-03-15"]]
    );
}

// A deliberate difference: PostgreSQL 17 skips punctuation after a date and reads both of these as
// 2024-03-15. This subset refuses any text after the date that is not a time or an `AD` era.
#[test]
fn a_trailing_comma_after_the_date_is_refused() {
    let mut f = fixture();
    let mut s = common::admin();
    for text in ["2024-03-15,", "2024-03-15 ,"] {
        let sql = format!("SELECT '{text}'::date");
        assert_eq!(err(&mut f.db, &mut s, &sql).sqlstate(), "22007", "{sql}");
    }
}

// PostgreSQL 17 reads `2024-03-15 10:00ad` as 2024-03-15 (it takes the era after the time with no
// space); this subset requires whitespace before the era, so the era is refused as stray text. The
// `10:00:00Zad` row matches PostgreSQL, which also refuses it.
#[test]
fn an_era_must_follow_whitespace() {
    let mut f = fixture();
    let mut s = common::admin();
    for text in ["2024-03-15 10:00:00Zad", "2024-03-15 10:00ad"] {
        let sql = format!("SELECT '{text}'::date");
        assert_eq!(err(&mut f.db, &mut s, &sql).sqlstate(), "22007", "{sql}");
    }
}
