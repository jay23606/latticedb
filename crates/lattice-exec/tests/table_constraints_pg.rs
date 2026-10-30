//! Column and key declarations checked against PostgreSQL 16: a bare `NULL` constraint, conflicting
//! `NULL` and `NOT NULL`, and a column named twice in one key. Each entry is the SQLSTATE PostgreSQL
//! gave, or `OK` when it created the table.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, ok, rows};

const CASES: &[(&str, &str)] = &[
    ("CREATE TABLE a1 (a INT NULL NOT NULL)", "42601"),
    ("CREATE TABLE a2 (a INT NOT NULL NULL)", "42601"),
    ("CREATE TABLE a3 (a INT NULL NULL)", "OK"),
    ("CREATE TABLE a4 (a INT NULL PRIMARY KEY)", "OK"),
    ("CREATE TABLE a5 (a INT PRIMARY KEY NULL)", "OK"),
    ("CREATE TABLE a6 (a INT, PRIMARY KEY (a, a))", "42701"),
    ("CREATE TABLE a7 (a INT, b INT, UNIQUE (a, b, a))", "42701"),
    (
        "CREATE TABLE a8 (a INT, b INT, UNIQUE (a, b), UNIQUE (b, a))",
        "OK",
    ),
    ("CREATE TABLE a9 (a INT NULL, b INT UNIQUE NULL)", "OK"),
];

#[test]
fn declarations_are_accepted_or_refused_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in CASES {
        match s.execute(&mut f.db, sql) {
            Ok(_) => assert_eq!(*want, "OK", "{sql}"),
            Err(e) => assert_eq!(e.sqlstate(), *want, "{sql}"),
        }
    }
}

#[test]
fn a_null_constraint_leaves_the_column_nullable_and_a_primary_key_still_forces_not_null() {
    let mut f = fixture();
    let mut s = common::admin();
    ok(
        &mut f.db,
        &mut s,
        "CREATE TABLE n (a INT NULL, b INT PRIMARY KEY NULL)",
    );
    ok(&mut f.db, &mut s, "INSERT INTO n VALUES (NULL, 1)");
    assert_eq!(
        rows(&mut f.db, &mut s, "SELECT a, b FROM n"),
        [["NULL", "1"]]
    );
    assert_eq!(
        common::err(&mut f.db, &mut s, "INSERT INTO n VALUES (1, NULL)").sqlstate(),
        "23502"
    );
}
