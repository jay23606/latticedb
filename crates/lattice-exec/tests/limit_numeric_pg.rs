//! `LIMIT` and `OFFSET` counts that are not integers, checked against PostgreSQL 16: a numeric or
//! floating point count is rounded to bigint (numeric rounds half away from zero, double precision
//! half to even), a negative count is refused after rounding, and a count past bigint is out of range.
//! Each step lists what PostgreSQL answered.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const STEPS: &[(&str, &[&str])] = &[
    ("CREATE TABLE o (id INT PRIMARY KEY)", &["CREATE TABLE"]),
    (
        "INSERT INTO o VALUES (1), (2), (3), (4), (5), (6)",
        &["INSERT 0 6"],
    ),
    ("SELECT id FROM o ORDER BY id LIMIT 1.5", &["1", "2"]),
    ("SELECT id FROM o ORDER BY id LIMIT 2.5", &["1", "2", "3"]),
    ("SELECT id FROM o ORDER BY id LIMIT 2.4", &["1", "2"]),
    ("SELECT id FROM o ORDER BY id LIMIT 2.0", &["1", "2"]),
    ("SELECT id FROM o ORDER BY id LIMIT 0.4", &[]),
    ("SELECT id FROM o ORDER BY id LIMIT 0.5", &["1"]),
    ("SELECT id FROM o ORDER BY id LIMIT -0.4", &[]),
    ("SELECT id FROM o ORDER BY id LIMIT -0.5", &["ERR 2201W"]),
    ("SELECT id FROM o ORDER BY id LIMIT -1.5", &["ERR 2201W"]),
    (
        "SELECT id FROM o ORDER BY id OFFSET 1.5",
        &["3", "4", "5", "6"],
    ),
    ("SELECT id FROM o ORDER BY id OFFSET 2.5 LIMIT 1", &["4"]),
    (
        "SELECT id FROM o ORDER BY id OFFSET -0.4",
        &["1", "2", "3", "4", "5", "6"],
    ),
    ("SELECT id FROM o ORDER BY id OFFSET -0.5", &["ERR 2201X"]),
    ("SELECT id FROM o ORDER BY id LIMIT 1e0", &["1"]),
    (
        "SELECT id FROM o ORDER BY id LIMIT 1e1",
        &["1", "2", "3", "4", "5", "6"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 2.5::float8",
        &["1", "2"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 3.5::float8",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM o ORDER BY id LIMIT 0.5::float8", &[]),
    (
        "SELECT id FROM o ORDER BY id LIMIT 1.5::numeric",
        &["1", "2"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 99999999999999999999",
        &["ERR 22003"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 9223372036854775807.4",
        &["1", "2", "3", "4", "5", "6"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 9223372036854775807.5",
        &["ERR 22003"],
    ),
    (
        "SELECT id FROM o ORDER BY id LIMIT 'NaN'::float8",
        &["ERR 22003"],
    ),
    ("SELECT id FROM o ORDER BY id LIMIT 1 + 0.5", &["1", "2"]),
    (
        "SELECT id FROM o ORDER BY id LIMIT 7 / 2.0",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM o ORDER BY id LIMIT 'abc'", &["ERR 22P02"]),
    ("SELECT id FROM o ORDER BY id LIMIT true", &["ERR 42804"]),
    ("SELECT id FROM o ORDER BY id LIMIT 3", &["1", "2", "3"]),
];

#[test]
fn fractional_row_counts_round_as_postgresql_rounds_them() {
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
