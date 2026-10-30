//! Text input forms checked against PostgreSQL 16: `bytea` in hex and escape form (bad digits, odd digit
//! counts, octal escapes, lone backslashes) and the underscores allowed between digits of integers and
//! numerics (`1_000`). Each entry lists what PostgreSQL answered: the rows or `ERR` and the SQLSTATE.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const CASES: &[(&str, &[&str])] = &[
    ("SELECT 'x'::bytea", &["\\x78"]),
    ("SELECT 'abc'::bytea", &["\\x616263"]),
    ("SELECT ''::bytea", &["\\x"]),
    ("SELECT '\\x'::bytea", &["\\x"]),
    ("SELECT '\\x4142'::bytea", &["\\x4142"]),
    ("SELECT '\\X4142'::bytea", &["ERR 22P02"]),
    ("SELECT '\\x41 42'::bytea", &["\\x4142"]),
    ("SELECT '\\x 4142 '::bytea", &["\\x4142"]),
    ("SELECT '\\x4 142'::bytea", &["ERR 22023"]),
    ("SELECT '\\x414'::bytea", &["ERR 22023"]),
    ("SELECT '\\xzz'::bytea", &["ERR 22023"]),
    ("SELECT '\\x4g'::bytea", &["ERR 22023"]),
    ("SELECT '\\xAbCd'::bytea", &["\\xabcd"]),
    ("SELECT ' \\x41'::bytea", &["ERR 22P02"]),
    ("SELECT '\\\\'::bytea", &["\\x5c"]),
    ("SELECT 'a\\\\b'::bytea", &["\\x615c62"]),
    ("SELECT '\\101'::bytea", &["\\x41"]),
    ("SELECT '\\000'::bytea", &["\\x00"]),
    ("SELECT '\\377'::bytea", &["\\xff"]),
    ("SELECT '\\400'::bytea", &["ERR 22P02"]),
    ("SELECT '\\18'::bytea", &["ERR 22P02"]),
    ("SELECT '\\1'::bytea", &["ERR 22P02"]),
    ("SELECT '\\12'::bytea", &["ERR 22P02"]),
    ("SELECT '\\'::bytea", &["ERR 22P02"]),
    ("SELECT 'a\\'::bytea", &["ERR 22P02"]),
    ("SELECT '\\n'::bytea", &["ERR 22P02"]),
    ("SELECT '\\\\101'::bytea", &["\\x5c313031"]),
    ("SELECT length('\\\\\\\\'::bytea)", &["2"]),
    ("SELECT '1_000'::int", &["1000"]),
    ("SELECT '1__000'::int", &["ERR 22P02"]),
    ("SELECT '_1'::int", &["ERR 22P02"]),
    ("SELECT '1_'::int", &["ERR 22P02"]),
    ("SELECT '1_0_0'::bigint", &["100"]),
    ("SELECT '-1_0'::int", &["-10"]),
    ("SELECT '1_000'::numeric", &["1000"]),
    ("SELECT '1_000.5_5'::numeric", &["1000.55"]),
    ("SELECT '1_000'::float8", &["ERR 22P02"]),
    ("SELECT '1_000'::smallint", &["1000"]),
    ("SELECT ' 1_0 '::int", &["10"]),
    ("SELECT '+1_0'::int", &["10"]),
    ("SELECT '1_.5'::numeric", &["ERR 22P02"]),
    ("SELECT '1._5'::numeric", &["ERR 22P02"]),
    ("SELECT '1e1_0'::numeric", &["10000000000"]),
    ("SELECT '3_000_000_000'::int", &["ERR 22003"]),
    ("SELECT '1000'::int", &["1000"]),
];

#[test]
fn input_forms_answer_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in CASES {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) => text(&r).iter().map(|row| row.join("|")).collect(),
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}
