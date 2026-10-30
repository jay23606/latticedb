//! Boolean, floating point, integer and bytea expressions checked against PostgreSQL 16: text input
//! and output, arithmetic edges, overflow and division errors, casts, `NULL` logic. Each entry lists
//! what PostgreSQL answered: the rows (`NULL` is an empty field) or `ERR` and the SQLSTATE.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const CASES: &[(&str, &[&str])] = &[
    ("SELECT true, false, NULL::boolean", &["t|f|"]),
    (
        "SELECT 't'::boolean, 'yes'::boolean, 'on'::boolean, '1'::boolean, 'TRUE'::boolean, ' f '::boolean",
        &["t|t|t|t|t|f"],
    ),
    (
        "SELECT 'n'::boolean, 'no'::boolean, 'off'::boolean, '0'::boolean",
        &["f|f|f|f"],
    ),
    ("SELECT 'maybe'::boolean", &["ERR 22P02"]),
    (
        "SELECT true AND NULL, false AND NULL, true OR NULL, false OR NULL, NOT NULL::boolean",
        &["|f|t||"],
    ),
    (
        "SELECT true = false, true <> false, true < false",
        &["f|t|f"],
    ),
    (
        "SELECT 1.5::float8, 0.1::float8, 1e10::float8, 1e15::float8, 1e16::float8, 123456789012345678::float8",
        &["1.5|0.1|10000000000|1e+15|1e+16|1.2345678901234568e+17"],
    ),
    (
        "SELECT 1e-5::float8, 0.0001::float8, 1.0::float8, -0.0::float8",
        &["1e-05|0.0001|1|-0"],
    ),
    (
        "SELECT 'NaN'::float8, 'Infinity'::float8, '-Infinity'::float8",
        &["NaN|Infinity|-Infinity"],
    ),
    (
        "SELECT 'NaN'::float8 = 'NaN'::float8, 'NaN'::float8 > 1::float8",
        &["t|t"],
    ),
    ("SELECT 1.0::float8 / 0", &["ERR 22012"]),
    ("SELECT 0.1::float8 + 0.2::float8", &["0.30000000000000004"]),
    ("SELECT 3.0::float8 * 1e308::float8", &["ERR 22003"]),
    (
        "SELECT 1.5::float8::int, 2.5::float8::int, (-1.5)::float8::int, 0.5::float8::int",
        &["2|2|-2|0"],
    ),
    ("SELECT 1e300::float8::int", &["ERR 22003"]),
    ("SELECT 'abc'::float8", &["ERR 22P02"]),
    (
        "SELECT '1e5'::float8, ' 12 '::float8, '+1.5'::float8, '.5'::float8, '5.'::float8",
        &["100000|12|1.5|0.5|5"],
    ),
    ("SELECT length('\\x00ff'::bytea)", &["2"]),
    (
        "SELECT '\\x01'::bytea = '\\x01'::bytea, '\\x01'::bytea < '\\x02'::bytea",
        &["t|t"],
    ),
    ("SELECT '\\x'::bytea || '\\x01'::bytea", &["\\x01"]),
    ("SELECT 2147483647::int + 1", &["ERR 22003"]),
    ("SELECT 2147483647::int * 2", &["ERR 22003"]),
    ("SELECT -2147483648::int - 1", &["ERR 22003"]),
    ("SELECT 9223372036854775807::bigint + 1", &["ERR 22003"]),
    ("SELECT (-9223372036854775808)::bigint - 1", &["ERR 22003"]),
    (
        "SELECT 7 / 2, -7 / 2, 7 % 3, -7 % 3, 7 % -3",
        &["3|-3|1|-1|1"],
    ),
    ("SELECT 1 / 0", &["ERR 22012"]),
    ("SELECT 1 % 0", &["ERR 22012"]),
    ("SELECT 5 / 2.0, 5 % 2.5", &["2.5000000000000000|0.0"]),
    (
        "SELECT abs(-5), abs(-5.5), abs(-2147483648::int)",
        &["ERR 22003"],
    ),
    ("SELECT 1 = 1.0, 1 < 1.5, 2 > 1.5", &["t|t|t"]),
    ("SELECT 32768::smallint", &["ERR 22003"]),
    ("SELECT 32767::smallint + 1::smallint", &["ERR 22003"]),
    ("SELECT '12abc'::int", &["ERR 22P02"]),
    ("SELECT ''::int", &["ERR 22P02"]),
    ("SELECT ' 12 '::int, '+7'::int, '-0'::int", &["12|7|0"]),
    ("SELECT '2147483648'::int", &["ERR 22003"]),
    ("SELECT '9223372036854775808'::bigint", &["ERR 22003"]),
    (
        "SELECT 1::text, 1.5::text, true::text, NULL::text",
        &["1|1.5|true|"],
    ),
    ("SELECT 'true'::text::boolean", &["t"]),
    ("SELECT 10::bool, 0::bool", &["t|f"]),
    ("SELECT 2::bool", &["t"]),
    ("SELECT 1.5::bool", &["ERR 42846"]),
    ("SELECT true::int, false::int", &["1|0"]),
    ("SELECT true::float8", &["ERR 42846"]),
    ("SELECT 'a' + 1", &["ERR 22P02"]),
    ("SELECT -'5'::int", &["-5"]),
    ("SELECT +5, - -5", &["5|5"]),
    ("SELECT coalesce(NULL, NULL), coalesce(NULL, 2, 3)", &["|2"]),
    (
        "SELECT nullif(1, 1), nullif(1, 2), nullif(NULL, 1)",
        &["|1|"],
    ),
];

#[test]
fn type_expressions_answer_as_postgresql_does() {
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
