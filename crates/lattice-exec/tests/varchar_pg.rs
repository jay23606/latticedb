//! `varchar(n)` assignment, checked against PostgreSQL 16: trailing spaces beyond the limit are
//! dropped silently, anything else beyond it is `22001`. Each step lists what PostgreSQL answered.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const STEPS: &[(&str, &[&str])] = &[
    ("CREATE TABLE v (a VARCHAR(3))", &["CREATE TABLE"]),
    ("INSERT INTO v VALUES ('abc')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('abcd')", &["ERR 22001"]),
    ("INSERT INTO v VALUES ('ab   ')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('abc   ')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('abc x')", &["ERR 22001"]),
    ("INSERT INTO v VALUES ('   ')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('     ')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('é  ')", &["INSERT 0 1"]),
    ("INSERT INTO v VALUES ('éé  é')", &["ERR 22001"]),
    ("INSERT INTO v VALUES ('abc\t')", &["ERR 22001"]),
    (
        "SELECT a, length(a) FROM v ORDER BY a, length(a)",
        &["   |3", "   |3", "ab |3", "abc|3", "abc|3", "é  |3"],
    ),
    ("UPDATE v SET a = 'xy    ' WHERE a = 'abc'", &["UPDATE 2"]),
    ("UPDATE v SET a = 'xyzw' WHERE a = 'xy '", &["ERR 22001"]),
    (
        "SELECT a, length(a) FROM v ORDER BY a, length(a)",
        &["   |3", "   |3", "ab |3", "xy |3", "xy |3", "é  |3"],
    ),
];

#[test]
fn varchar_limits_truncate_only_trailing_spaces() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in STEPS {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) if !r.columns.is_empty() => text(&r).iter().map(|row| row.join("|")).collect(),
            Ok(r) => vec![r.tag],
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}
