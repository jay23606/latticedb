//! Inserts, updates, deletes and DDL against a table with a primary key, a unique column, a default and
//! a `NOT NULL` column, in one session, checked against PostgreSQL 16. Each step lists what PostgreSQL
//! answered: the command tag, `ERR` and the SQLSTATE, or the rows (`NULL` is an empty field).
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
        "CREATE TABLE d (id INT PRIMARY KEY, n INT NOT NULL DEFAULT 7, s TEXT, u INT UNIQUE)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d VALUES (1, 1, 'a', 10)", &["INSERT 0 1"]),
    ("INSERT INTO d VALUES (1, 2, 'b', 11)", &["ERR 23505"]),
    ("INSERT INTO d VALUES (2, NULL, 'b', 11)", &["ERR 23502"]),
    ("INSERT INTO d (id, s) VALUES (2, 'x')", &["INSERT 0 1"]),
    (
        "SELECT id, n, s, u FROM d ORDER BY id",
        &["1|1|a|10", "2|7|x|"],
    ),
    (
        "INSERT INTO d (id, n, s, u) VALUES (3, 3, 'c', 10)",
        &["ERR 23505"],
    ),
    ("INSERT INTO d (id) VALUES (3), (4)", &["INSERT 0 2"]),
    ("INSERT INTO d VALUES (5, 5)", &["INSERT 0 1"]),
    ("INSERT INTO d VALUES (5, 5, 'a', 1, 2)", &["ERR 42601"]),
    ("INSERT INTO d (id, id) VALUES (6, 6)", &["ERR 42701"]),
    ("INSERT INTO d (zzz) VALUES (1)", &["ERR 42703"]),
    ("INSERT INTO d VALUES ('x', 1, 'a', 1)", &["ERR 22P02"]),
    (
        "INSERT INTO d VALUES (9999999999, 1, 'a', 1)",
        &["ERR 22003"],
    ),
    ("INSERT INTO d VALUES (6, 1.5, 'a', 12)", &["INSERT 0 1"]),
    (
        "INSERT INTO d (id, s, u) VALUES (7, NULL, NULL), (8, NULL, NULL)",
        &["INSERT 0 2"],
    ),
    (
        "SELECT id, n, s, u FROM d ORDER BY id",
        &[
            "1|1|a|10", "2|7|x|", "3|7||", "4|7||", "5|5||", "6|2|a|12", "7|7||", "8|7||",
        ],
    ),
    ("UPDATE d SET n = n + 1 WHERE id = 1", &["UPDATE 1"]),
    ("UPDATE d SET n = NULL WHERE id = 1", &["ERR 23502"]),
    ("UPDATE d SET id = 2 WHERE id = 1", &["ERR 23505"]),
    ("UPDATE d SET u = 12 WHERE id IN (1, 2)", &["ERR 23505"]),
    ("UPDATE d SET s = 'z' WHERE id > 100", &["UPDATE 0"]),
    (
        "UPDATE d SET s = s || '!' WHERE s IS NOT NULL",
        &["UPDATE 3"],
    ),
    ("UPDATE d SET zzz = 1", &["ERR 42703"]),
    ("UPDATE d SET n = 1, n = 2 WHERE id = 1", &["ERR 42601"]),
    ("UPDATE d SET id = id + 100 WHERE id = 8", &["UPDATE 1"]),
    (
        "SELECT id, n, s, u FROM d ORDER BY id",
        &[
            "1|2|a!|10",
            "2|7|x!|",
            "3|7||",
            "4|7||",
            "5|5||",
            "6|2|a!|12",
            "7|7||",
            "108|7||",
        ],
    ),
    ("DELETE FROM d WHERE id = 108", &["DELETE 1"]),
    ("DELETE FROM d WHERE id = 108", &["DELETE 0"]),
    ("DELETE FROM d WHERE u IS NULL", &["DELETE 5"]),
    ("DELETE FROM d WHERE zzz = 1", &["ERR 42703"]),
    ("INSERT INTO d VALUES (3, 3, 'c', 10) ", &["ERR 23505"]),
    (
        "SELECT id, n, s, u FROM d ORDER BY id",
        &["1|2|a!|10", "6|2|a!|12"],
    ),
    ("DELETE FROM d", &["DELETE 2"]),
    ("INSERT INTO nope VALUES (1)", &["ERR 42P01"]),
    ("UPDATE nope SET a = 1", &["ERR 42P01"]),
    ("DELETE FROM nope", &["ERR 42P01"]),
    ("CREATE TABLE d (id INT)", &["ERR 42P07"]),
    ("CREATE TABLE IF NOT EXISTS d (id INT)", &["CREATE TABLE"]),
    ("DROP TABLE nope", &["ERR 42P01"]),
    ("DROP TABLE IF EXISTS nope", &["DROP TABLE"]),
    ("DROP TABLE d", &["DROP TABLE"]),
    (
        "CREATE TABLE e (a INT, b INT, PRIMARY KEY (a, b))",
        &["CREATE TABLE"],
    ),
    (
        "INSERT INTO e VALUES (1, 1), (1, 2), (2, 1)",
        &["INSERT 0 3"],
    ),
    ("INSERT INTO e VALUES (1, 2)", &["ERR 23505"]),
    ("INSERT INTO e VALUES (1, NULL)", &["ERR 23502"]),
    ("SELECT a, b FROM e ORDER BY a, b", &["1|1", "1|2", "2|1"]),
    (
        "CREATE TABLE f (id INT PRIMARY KEY, id2 INT PRIMARY KEY)",
        &["ERR 42P16"],
    ),
    ("CREATE TABLE g (id INT, id INT)", &["ERR 42701"]),
    ("CREATE TABLE i (a INT NOT NULL NULL)", &["ERR 42601"]),
];

#[test]
fn statements_answer_as_postgresql_does() {
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
