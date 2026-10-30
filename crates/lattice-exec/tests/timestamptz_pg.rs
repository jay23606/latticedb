//! `timestamptz` text input, checked against PostgreSQL 17: zone offsets of every spelling the subset
//! accepts (hours alone, `h`, `hh`, `hh:mm`, and colon-less runs of three or more digits, which
//! PostgreSQL reads as one number whose last two digits are the minutes; the largest offset is 15:59
//! either way, and `22009` beyond it or for sixty minutes), and fractions of a second with more than
//! six digits, which round to the nearest microsecond and may carry into the next second. Spellings
//! this executor does not accept (zone names, offsets with seconds or a space, a year past 9999) are
//! left out.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{admin, fixture, text};

/// (statement, the value PostgreSQL 17 prints, or `ERROR <sqlstate>`)
#[rustfmt::skip]
const CASES: [(&str, &str); 95] = [
    ("SELECT '2024-03-01 12:30:45+00'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+00:00'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+0000'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+1'::timestamptz", "2024-03-01 11:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+01'::timestamptz", "2024-03-01 11:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+9'::timestamptz", "2024-03-01 03:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+10'::timestamptz", "2024-03-01 02:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+15'::timestamptz", "2024-02-29 21:30:45+00"),
    ("SELECT '2024-03-01 12:30:45+15:59'::timestamptz", "2024-02-29 20:31:45+00"),
    ("SELECT '2024-03-01 12:30:45+15:60'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+16'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+16:00'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+16:01'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+17'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+23'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+24'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+24:00'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+25'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+99'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+100'::timestamptz", "2024-03-01 11:30:45+00"),
    ("SELECT '2024-03-01 12:30:45-15'::timestamptz", "2024-03-02 03:30:45+00"),
    ("SELECT '2024-03-01 12:30:45-15:59'::timestamptz", "2024-03-02 04:29:45+00"),
    ("SELECT '2024-03-01 12:30:45-16'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45-16:00'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45-24'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45-00:30'::timestamptz", "2024-03-01 13:00:45+00"),
    ("SELECT '2024-03-01 12:30:45-0:30'::timestamptz", "2024-03-01 13:00:45+00"),
    ("SELECT '2024-03-01 12:30:45+05:3'::timestamptz", "2024-03-01 07:27:45+00"),
    ("SELECT '2024-03-01 12:30:45+05:03'::timestamptz", "2024-03-01 07:27:45+00"),
    ("SELECT '2024-03-01 12:30:45+5:30'::timestamptz", "2024-03-01 07:00:45+00"),
    ("SELECT '2024-03-01 12:30:45+0530'::timestamptz", "2024-03-01 07:00:45+00"),
    ("SELECT '2024-03-01 12:30:45+053'::timestamptz", "2024-03-01 11:37:45+00"),
    ("SELECT '2024-03-01 12:30:45+05300'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+00530'::timestamptz", "2024-03-01 07:00:45+00"),
    ("SELECT '2024-03-01 12:30:45-00530'::timestamptz", "2024-03-01 18:00:45+00"),
    ("SELECT '2024-03-01 12:30:45+05030'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+1530'::timestamptz", "2024-02-29 21:00:45+00"),
    ("SELECT '2024-03-01 12:30:45+1559'::timestamptz", "2024-02-29 20:31:45+00"),
    ("SELECT '2024-03-01 12:30:45+1560'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+1600'::timestamptz", "ERROR 22009"),
    ("SELECT '2024-03-01 12:30:45+:30'::timestamptz", "ERROR 22007"),
    ("SELECT '2024-03-01 12:30:45+aa'::timestamptz", "ERROR 22007"),
    ("SELECT '2024-03-01 12:30:45+05:aa'::timestamptz", "ERROR 22007"),
    ("SELECT '2024-03-01 12:30:45-00'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45Z'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45z'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45 +05:30'::timestamptz", "2024-03-01 07:00:45+00"),
    ("SELECT '2024-03-01 12:30:45.1234565'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.1234575'::timestamptz", "2024-03-01 12:30:45.123458+00"),
    ("SELECT '2024-03-01 12:30:45.1234555'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.1234545'::timestamptz", "2024-03-01 12:30:45.123454+00"),
    ("SELECT '2024-03-01 12:30:45.12345649'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.12345650'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.1234564999'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.9999995'::timestamptz", "2024-03-01 12:30:46+00"),
    ("SELECT '2024-03-01 12:30:45.9999994'::timestamptz", "2024-03-01 12:30:45.999999+00"),
    ("SELECT '2024-03-01 12:30:45.0000005'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45.0000004'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45.5000005'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.1'::timestamptz", "2024-03-01 12:30:45.1+00"),
    ("SELECT '2024-03-01 12:30:45.12'::timestamptz", "2024-03-01 12:30:45.12+00"),
    ("SELECT '2024-03-01 12:30:45.123'::timestamptz", "2024-03-01 12:30:45.123+00"),
    ("SELECT '2024-03-01 12:30:45.1234'::timestamptz", "2024-03-01 12:30:45.1234+00"),
    ("SELECT '2024-03-01 12:30:45.12345'::timestamptz", "2024-03-01 12:30:45.12345+00"),
    ("SELECT '2024-03-01 12:30:45.123456'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:45.1234567'::timestamptz", "2024-03-01 12:30:45.123457+00"),
    ("SELECT '2024-03-01 12:30:45.9999999'::timestamptz", "2024-03-01 12:30:46+00"),
    ("SELECT '2024-03-01 12:30:45.99999999999999999999'::timestamptz", "2024-03-01 12:30:46+00"),
    ("SELECT '2024-03-01 12:30:45.0000001'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45.0000009'::timestamptz", "2024-03-01 12:30:45.000001+00"),
    ("SELECT '2024-03-01 12:30:45.000000'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45.5'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.50'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.500000'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.5000005'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.4999995'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.4999996'::timestamptz", "2024-03-01 12:30:45.5+00"),
    ("SELECT '2024-03-01 12:30:45.2500005'::timestamptz", "2024-03-01 12:30:45.25+00"),
    ("SELECT '2024-03-01 12:30:45.7500005'::timestamptz", "2024-03-01 12:30:45.75+00"),
    ("SELECT '2024-03-01 12:30:45.3333335'::timestamptz", "2024-03-01 12:30:45.333334+00"),
    ("SELECT '2024-03-01 12:30:45.6666665'::timestamptz", "2024-03-01 12:30:45.666666+00"),
    ("SELECT '2024-03-01 12:30:45.1000005'::timestamptz", "2024-03-01 12:30:45.1+00"),
    ("SELECT '2024-03-01 12:30:45.0000015'::timestamptz", "2024-03-01 12:30:45.000002+00"),
    ("SELECT '2024-03-01 12:30:45.0000025'::timestamptz", "2024-03-01 12:30:45.000002+00"),
    ("SELECT '2024-03-01 12:30:45.0000035'::timestamptz", "2024-03-01 12:30:45.000004+00"),
    ("SELECT '2024-03-01 12:30:45.1234500'::timestamptz", "2024-03-01 12:30:45.12345+00"),
    ("SELECT '2024-03-01 12:30:45.1234550'::timestamptz", "2024-03-01 12:30:45.123455+00"),
    ("SELECT '2024-03-01 12:30:45.1234549'::timestamptz", "2024-03-01 12:30:45.123455+00"),
    ("SELECT '2024-03-01 12:30:45.12345451'::timestamptz", "2024-03-01 12:30:45.123455+00"),
    ("SELECT '2024-03-01 12:30:45.123456000000'::timestamptz", "2024-03-01 12:30:45.123456+00"),
    ("SELECT '2024-03-01 12:30:59.9999995'::timestamptz", "2024-03-01 12:31:00+00"),
    ("SELECT '2024-12-31 23:59:59.9999995'::timestamptz", "2025-01-01 00:00:00+00"),
    ("SELECT '2024-03-01 12:30:45.'::timestamptz", "2024-03-01 12:30:45+00"),
    ("SELECT '2024-03-01 12:30:45.x'::timestamptz", "ERROR 22007"),
    ("SELECT '2024-03-01 12:30:60.9999995'::timestamptz", "2024-03-01 12:31:01+00"),
];

#[test]
fn offsets_and_fractions_agree_with_postgresql() {
    let mut f = fixture();
    let mut s = admin();
    for (sql, expected) in CASES {
        let got = match s.execute(&mut f.db, sql) {
            Ok(r) => text(&r)[0][0].clone(),
            Err(e) => format!("ERROR {}", e.sqlstate()),
        };
        assert_eq!(got, expected, "{sql}");
    }
}
