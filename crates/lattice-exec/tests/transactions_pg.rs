//! Transaction control in one session, checked against PostgreSQL 16: rollback, commit, a failed
//! statement poisoning the block (`25P02`), a nested `BEGIN`, and `COMMIT` or `ROLLBACK` outside a
//! block. Each step lists what PostgreSQL answered: the command tag, `ERR` and the SQLSTATE, or rows.
//! Warnings are not compared. Savepoints, `END`, transaction modes and DDL inside a block are not
//! executed yet and are left out.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const STEPS: &[(&str, &[&str])] = &[
    (
        "CREATE TABLE x (id INT PRIMARY KEY, v INT)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO x VALUES (1, 1)", &["INSERT 0 1"]),
    ("BEGIN", &["BEGIN"]),
    ("INSERT INTO x VALUES (2, 2)", &["INSERT 0 1"]),
    ("SELECT id, v FROM x ORDER BY id", &["1|1", "2|2"]),
    ("ROLLBACK", &["ROLLBACK"]),
    ("SELECT id, v FROM x ORDER BY id", &["1|1"]),
    ("BEGIN", &["BEGIN"]),
    ("UPDATE x SET v = 10 WHERE id = 1", &["UPDATE 1"]),
    ("COMMIT", &["COMMIT"]),
    ("SELECT id, v FROM x ORDER BY id", &["1|10"]),
    ("BEGIN", &["BEGIN"]),
    ("INSERT INTO x VALUES (1, 5)", &["ERR 23505"]),
    ("SELECT 1", &["ERR 25P02"]),
    ("INSERT INTO x VALUES (3, 3)", &["ERR 25P02"]),
    ("COMMIT", &["ROLLBACK"]),
    ("SELECT id, v FROM x ORDER BY id", &["1|10"]),
    ("BEGIN", &["BEGIN"]),
    ("BEGIN", &["BEGIN"]),
    ("ROLLBACK", &["ROLLBACK"]),
    ("ROLLBACK", &["ROLLBACK"]),
    ("COMMIT", &["COMMIT"]),
    ("BEGIN", &["BEGIN"]),
    ("SELECT 1/0", &["ERR 22012"]),
    ("SELECT 2", &["ERR 25P02"]),
    ("ROLLBACK", &["ROLLBACK"]),
    ("BEGIN ISOLATION LEVEL SERIALIZABLE", &["BEGIN"]),
    ("COMMIT", &["COMMIT"]),
];

#[test]
fn transaction_statements_answer_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in STEPS {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) if !r.columns.is_empty() => text(&r)
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| if c == "NULL" { "" } else { c.as_str() })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect(),
            Ok(r) => vec![r.tag],
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}
