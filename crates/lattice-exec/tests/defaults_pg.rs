//! Column defaults and `public`-qualified table names in one session, checked against PostgreSQL 16:
//! signed, `NULL`, cast and typed literal defaults, defaults that do not fit or do not parse, and the
//! `public.` prefix. Each step lists what PostgreSQL answered. Left out on purpose: a default that
//! overflows its column (PostgreSQL accepts the table and fails at the first insert; this server
//! refuses the table), an unknown schema on a read (`42P01` there, `3F000` here) and a fully
//! qualified column reference (`public.t.c`, not executed yet).
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
        "CREATE TABLE d1 (id INT PRIMARY KEY, a INT DEFAULT -5, b INT DEFAULT +5, c INT DEFAULT NULL, d DOUBLE PRECISION DEFAULT -1.5, e NUMERIC DEFAULT -2.50, f NUMERIC DEFAULT +0.5, g BIGINT DEFAULT -9223372036854775807)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d1 (id) VALUES (1)", &["INSERT 0 1"]),
    (
        "SELECT a, b, c, d, e, f, g FROM d1",
        &["-5|5||-1.5|-2.50|0.5|-9223372036854775807"],
    ),
    (
        "CREATE TABLE d2 (id INT PRIMARY KEY, a INT DEFAULT CAST('7' AS INT), b INT DEFAULT '8'::int, c TEXT DEFAULT 'x'::text, d BOOLEAN DEFAULT true, e BOOLEAN DEFAULT 'f', f DOUBLE PRECISION DEFAULT 2, g NUMERIC DEFAULT 3)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d2 (id) VALUES (1)", &["INSERT 0 1"]),
    ("SELECT a, b, c, d, e, f, g FROM d2", &["7|8|x|t|f|2|3"]),
    (
        "CREATE TABLE d3 (id INT PRIMARY KEY, a INT DEFAULT 1.5)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d3 (id) VALUES (1)", &["INSERT 0 1"]),
    ("SELECT a FROM d3", &["2"]),
    (
        "CREATE TABLE d6 (id INT PRIMARY KEY, a INT DEFAULT 'abc')",
        &["ERR 22P02"],
    ),
    (
        "CREATE TABLE d7 (id INT PRIMARY KEY, a INT NOT NULL DEFAULT NULL)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d7 (id) VALUES (1)", &["ERR 23502"]),
    (
        "CREATE TABLE d8 (id INT PRIMARY KEY, a TEXT DEFAULT NULL, b INT DEFAULT - -3)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d8 (id) VALUES (1)", &["INSERT 0 1"]),
    ("SELECT a, b FROM d8", &["|3"]),
    (
        "CREATE TABLE d9 (id INT PRIMARY KEY, a INT DEFAULT 1 DEFAULT 2)",
        &["ERR 42601"],
    ),
    (
        "CREATE TABLE d10 (id INT PRIMARY KEY, a UUID DEFAULT '00000000-0000-0000-0000-000000000001')",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d10 (id) VALUES (1)", &["INSERT 0 1"]),
    (
        "SELECT a FROM d10",
        &["00000000-0000-0000-0000-000000000001"],
    ),
    (
        "CREATE TABLE d11 (id INT PRIMARY KEY, a BYTEA DEFAULT '\\x0102')",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO d11 (id) VALUES (1)", &["INSERT 0 1"]),
    ("SELECT a FROM d11", &["\\x0102"]),
    ("CREATE TABLE public.p1 (id INT)", &["CREATE TABLE"]),
    ("INSERT INTO public.p1 VALUES (1)", &["INSERT 0 1"]),
    ("SELECT id FROM public.p1", &["1"]),
    ("SELECT id FROM p1", &["1"]),
    ("CREATE TABLE other.o1 (id INT)", &["ERR 3F000"]),
    ("CREATE TABLE a.b.c (id INT)", &["ERR 0A000"]),
    ("SELECT p1.id FROM public.p1", &["1"]),
    ("DROP TABLE public.p1", &["DROP TABLE"]),
    ("DROP TABLE public.p1", &["ERR 42P01"]),
];

#[test]
fn defaults_and_public_names_answer_as_postgresql_does() {
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
