//! `date_trunc`, `extract`, date and timestamptz casts and comparisons checked against PostgreSQL 16
//! with the session time zone at UTC: every unit both support, dates before 1970, week and quarter
//! boundaries, leap days and out-of-range dates. Each entry lists what PostgreSQL answered. Left out
//! on purpose and filed as issues: the units this server does not execute yet, `extract(epoch FROM
//! date)` (PostgreSQL prints no decimals), date arithmetic, and a `BC` date.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const CASES: &[(&str, &[&str])] = &[
    (
        "SELECT date_trunc('year', '2024-03-15 10:20:30.123456+00'::timestamptz)",
        &["2024-01-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('quarter', '2024-05-15 10:20:30+00'::timestamptz)",
        &["2024-04-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('quarter', '2024-12-31 23:59:59+00'::timestamptz)",
        &["2024-10-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('month', '2024-03-15 10:20:30+00'::timestamptz)",
        &["2024-03-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('week', '2024-03-15 10:20:30+00'::timestamptz)",
        &["2024-03-11 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('week', '2024-03-17 10:20:30+00'::timestamptz)",
        &["2024-03-11 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('week', '2024-03-18 00:00:00+00'::timestamptz)",
        &["2024-03-18 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('day', '2024-03-15 10:20:30+00'::timestamptz)",
        &["2024-03-15 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('hour', '2024-03-15 10:20:30+00'::timestamptz)",
        &["2024-03-15 10:00:00+00"],
    ),
    (
        "SELECT date_trunc('minute', '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["2024-03-15 10:20:00+00"],
    ),
    (
        "SELECT date_trunc('second', '2024-03-15 10:20:30.789+00'::timestamptz)",
        &["2024-03-15 10:20:30+00"],
    ),
    (
        "SELECT date_trunc('year', '1969-07-20 20:17:40+00'::timestamptz)",
        &["1969-01-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('month', '1969-07-20 20:17:40+00'::timestamptz)",
        &["1969-07-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('day', '1969-12-31 23:59:59.999999+00'::timestamptz)",
        &["1969-12-31 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('hour', '1969-12-31 23:59:59.999999+00'::timestamptz)",
        &["1969-12-31 23:00:00+00"],
    ),
    (
        "SELECT date_trunc('week', '1969-12-31 23:59:59+00'::timestamptz)",
        &["1969-12-29 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('quarter', '1969-02-01 00:00:00+00'::timestamptz)",
        &["1969-01-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('year', '0001-06-01 00:00:00+00'::timestamptz)",
        &["0001-01-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('day', '2024-03-15'::date)",
        &["2024-03-15 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('month', '2024-03-15'::date)",
        &["2024-03-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('YEAR', '2024-03-15 10:20:30+00'::timestamptz)",
        &["2024-01-01 00:00:00+00"],
    ),
    (
        "SELECT date_trunc('fortnight', '2024-03-15 10:20:30+00'::timestamptz)",
        &["ERR 22023"],
    ),
    (
        "SELECT date_trunc(NULL, '2024-03-15 10:20:30+00'::timestamptz)",
        &[""],
    ),
    ("SELECT date_trunc('day', NULL::timestamptz)", &[""]),
    (
        "SELECT extract(year FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["2024"],
    ),
    (
        "SELECT extract(month FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["3"],
    ),
    (
        "SELECT extract(day FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["15"],
    ),
    (
        "SELECT extract(hour FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["10"],
    ),
    (
        "SELECT extract(minute FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["20"],
    ),
    (
        "SELECT extract(second FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["30.500000"],
    ),
    (
        "SELECT extract(epoch FROM '2024-03-15 10:20:30.5+00'::timestamptz)",
        &["1710498030.500000"],
    ),
    (
        "SELECT extract(epoch FROM '1969-12-31 23:59:59+00'::timestamptz)",
        &["-1.000000"],
    ),
    (
        "SELECT extract(dow FROM '2024-03-17 10:20:30+00'::timestamptz)",
        &["0"],
    ),
    (
        "SELECT extract(dow FROM '2024-03-16 10:20:30+00'::timestamptz)",
        &["6"],
    ),
    (
        "SELECT extract(doy FROM '2024-03-15 10:20:30+00'::timestamptz)",
        &["75"],
    ),
    (
        "SELECT extract(doy FROM '2024-12-31 10:20:30+00'::timestamptz)",
        &["366"],
    ),
    (
        "SELECT extract(quarter FROM '2024-05-15 10:20:30+00'::timestamptz)",
        &["2"],
    ),
    (
        "SELECT extract(week FROM '2024-03-15 10:20:30+00'::timestamptz)",
        &["11"],
    ),
    (
        "SELECT extract(week FROM '2024-12-30 10:20:30+00'::timestamptz)",
        &["1"],
    ),
    (
        "SELECT extract(week FROM '2021-01-03 10:20:30+00'::timestamptz)",
        &["53"],
    ),
    (
        "SELECT extract(year FROM '1969-07-20 20:17:40+00'::timestamptz)",
        &["1969"],
    ),
    (
        "SELECT extract(month FROM '1969-07-20 20:17:40+00'::timestamptz)",
        &["7"],
    ),
    (
        "SELECT extract(day FROM '1969-12-31 23:59:59+00'::timestamptz)",
        &["31"],
    ),
    (
        "SELECT extract(hour FROM '1969-12-31 23:59:59.5+00'::timestamptz)",
        &["23"],
    ),
    (
        "SELECT extract(second FROM '1969-12-31 23:59:59.5+00'::timestamptz)",
        &["59.500000"],
    ),
    (
        "SELECT extract(dow FROM '1969-12-31 23:59:59+00'::timestamptz)",
        &["3"],
    ),
    (
        "SELECT extract(doy FROM '1969-12-31 23:59:59+00'::timestamptz)",
        &["365"],
    ),
    ("SELECT extract(year FROM '2024-03-15'::date)", &["2024"]),
    ("SELECT extract(dow FROM '2024-03-15'::date)", &["5"]),
    ("SELECT extract(year FROM '0044-03-15'::date)", &["44"]),
    ("SELECT extract(YEAR FROM '2024-03-15'::date)", &["2024"]),
    (
        "SELECT extract(fortnight FROM '2024-03-15'::date)",
        &["ERR 22023"],
    ),
    ("SELECT extract(year FROM NULL::date)", &[""]),
    (
        "SELECT '2024-03-15'::date < '2024-03-16'::date, '2024-03-15'::date = '2024-03-15'::date",
        &["t|t"],
    ),
    (
        "SELECT '2024-03-15'::date::timestamptz",
        &["2024-03-15 00:00:00+00"],
    ),
    (
        "SELECT '2024-03-15 23:59:59+00'::timestamptz::date",
        &["2024-03-15"],
    ),
    (
        "SELECT '1969-12-31 23:59:59+00'::timestamptz::date",
        &["1969-12-31"],
    ),
    (
        "SELECT '2024-03-15 10:20:30+00'::timestamptz = '2024-03-15 12:20:30+02'::timestamptz",
        &["t"],
    ),
    (
        "SELECT '2024-03-15'::date = '2024-03-15 00:00:00+00'::timestamptz",
        &["t"],
    ),
    (
        "SELECT '2024-03-15'::date < '2024-03-15 00:00:01+00'::timestamptz",
        &["t"],
    ),
    (
        "SELECT '2024-02-29'::date, '2023-02-29'::date",
        &["ERR 22008"],
    ),
    ("SELECT '2024-13-01'::date", &["ERR 22008"]),
    ("SELECT '0000-01-01'::date", &["ERR 22008"]),
    ("SELECT '5874898-01-01'::date", &["ERR 22008"]),
];

#[test]
fn date_and_time_expressions_answer_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in CASES {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) => text(&r)
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| if c == "NULL" { "" } else { c.as_str() })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect(),
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}
