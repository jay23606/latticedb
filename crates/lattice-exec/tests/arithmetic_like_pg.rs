//! Arithmetic over smallint-to-bigint, double precision and numeric columns, and `LIKE` with wildcards
//! and escapes, in one session, checked against PostgreSQL 16: signs of quotients and remainders, mixed
//! types, overflow at the edges of each type, and patterns that need backtracking. Each entry lists what
//! PostgreSQL answered. Left out on purpose: `double precision % x` (PostgreSQL has no such operator,
//! this server computes it).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, ok, text};

const SETUP: &[&str] = &[
    "CREATE TABLE ar (id INT PRIMARY KEY, i INT, b BIGINT, f DOUBLE PRECISION, n NUMERIC, s TEXT)",
    "INSERT INTO ar VALUES (1, 7, 7, 7.5, 7.5, 'abcabc'), (2, -7, -7, -7.5, -7.5, 'a%c_d'), (3, 0, 0, 0.0, 0, ''), (4, 2147483647, 9223372036854775807, '1e308'::float8, 99999999999999999999, 'ABC')",
];

const CASES: &[(&str, &[&str])] = &[
    (
        "SELECT id, i + 3 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|10", "2|-4", "3|3"],
    ),
    (
        "SELECT id, i - 10 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|-3", "2|-17", "3|-10"],
    ),
    (
        "SELECT id, i * -2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|-14", "2|14", "3|0"],
    ),
    (
        "SELECT id, i / 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|3", "2|-3", "3|0"],
    ),
    (
        "SELECT id, i / -2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|-3", "2|3", "3|0"],
    ),
    (
        "SELECT id, i % 3 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|1", "2|-1", "3|0"],
    ),
    (
        "SELECT id, i % -3 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|1", "2|-1", "3|0"],
    ),
    (
        "SELECT id, -i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|-7", "2|7", "3|0"],
    ),
    (
        "SELECT id, b + 1 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|8", "2|-6", "3|1"],
    ),
    (
        "SELECT id, b - 1 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|6", "2|-8", "3|-1"],
    ),
    (
        "SELECT id, b * 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|14", "2|-14", "3|0"],
    ),
    (
        "SELECT id, b / 3 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|2", "2|-2", "3|0"],
    ),
    (
        "SELECT id, b % 4 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|3", "2|-3", "3|0"],
    ),
    (
        "SELECT id, f + 1.5 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|9", "2|-6", "3|1.5"],
    ),
    (
        "SELECT id, f - 1.5 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|6", "2|-9", "3|-1.5"],
    ),
    (
        "SELECT id, f * 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|15", "2|-15", "3|0"],
    ),
    (
        "SELECT id, f / 4 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|1.875", "2|-1.875", "3|0"],
    ),
    (
        "SELECT id, n + 1 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|8.5", "2|-6.5", "3|1"],
    ),
    (
        "SELECT id, n - 1.25 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|6.25", "2|-8.75", "3|-1.25"],
    ),
    (
        "SELECT id, n * 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|15.0", "2|-15.0", "3|0"],
    ),
    (
        "SELECT id, n / 4 FROM ar WHERE id <= 3 ORDER BY id",
        &[
            "1|1.8750000000000000",
            "2|-1.8750000000000000",
            "3|0.00000000000000000000",
        ],
    ),
    (
        "SELECT id, n % 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|1.5", "2|-1.5", "3|0"],
    ),
    (
        "SELECT id, i + f FROM ar WHERE id <= 3 ORDER BY id",
        &["1|14.5", "2|-14.5", "3|0"],
    ),
    (
        "SELECT id, i * n FROM ar WHERE id <= 3 ORDER BY id",
        &["1|52.5", "2|52.5", "3|0"],
    ),
    (
        "SELECT id, f * n FROM ar WHERE id <= 3 ORDER BY id",
        &["1|56.25", "2|56.25", "3|0"],
    ),
    (
        "SELECT id, b * i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|49", "2|49", "3|0"],
    ),
    (
        "SELECT id, i + b FROM ar WHERE id <= 3 ORDER BY id",
        &["1|14", "2|-14", "3|0"],
    ),
    (
        "SELECT id, f / 0.5 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|15", "2|-15", "3|0"],
    ),
    (
        "SELECT id, n / 3 FROM ar WHERE id <= 3 ORDER BY id",
        &[
            "1|2.5000000000000000",
            "2|-2.5000000000000000",
            "3|0.00000000000000000000",
        ],
    ),
    (
        "SELECT id, i / 3.0 FROM ar WHERE id <= 3 ORDER BY id",
        &[
            "1|2.3333333333333333",
            "2|-2.3333333333333333",
            "3|0.00000000000000000000",
        ],
    ),
    (
        "SELECT id, b % 2.5 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|2.0", "2|-2.0", "3|0.0"],
    ),
    (
        "SELECT id, 1 + i * 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|15", "2|-13", "3|1"],
    ),
    (
        "SELECT id, (i + 1) * 2 FROM ar WHERE id <= 3 ORDER BY id",
        &["1|16", "2|-12", "3|2"],
    ),
    (
        "SELECT id, i - -i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|14", "2|-14", "3|0"],
    ),
    (
        "SELECT id, - -i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|7", "2|-7", "3|0"],
    ),
    (
        "SELECT id, -(i + 1) FROM ar WHERE id <= 3 ORDER BY id",
        &["1|-8", "2|6", "3|-1"],
    ),
    (
        "SELECT id, i * i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|49", "2|49", "3|0"],
    ),
    (
        "SELECT id, b * b FROM ar WHERE id <= 3 ORDER BY id",
        &["1|49", "2|49", "3|0"],
    ),
    (
        "SELECT id, i + i FROM ar WHERE id <= 3 ORDER BY id",
        &["1|14", "2|-14", "3|0"],
    ),
    (
        "SELECT id, f * f FROM ar WHERE id <= 3 ORDER BY id",
        &["1|56.25", "2|56.25", "3|0"],
    ),
    (
        "SELECT id, 10 % i FROM ar WHERE id <= 3 ORDER BY id",
        &["ERR 22012"],
    ),
    (
        "SELECT id, 10 / i FROM ar WHERE id <= 3 ORDER BY id",
        &["ERR 22012"],
    ),
    (
        "SELECT id, 10.0 / n FROM ar WHERE id <= 3 ORDER BY id",
        &["ERR 22012"],
    ),
    (
        "SELECT id, f - f FROM ar WHERE id <= 3 ORDER BY id",
        &["1|0", "2|0", "3|0"],
    ),
    (
        "SELECT id, n * n * n FROM ar WHERE id <= 3 ORDER BY id",
        &["1|421.875", "2|-421.875", "3|0"],
    ),
    ("SELECT id, i + 1 FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id, i * 2 FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id, b + 1 FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id, b * 2 FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id, f * 10 FROM ar WHERE id = 4", &["ERR 22003"]),
    (
        "SELECT id, n * 10 FROM ar WHERE id = 4",
        &["4|999999999999999999990"],
    ),
    ("SELECT id, -b - 2 FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id, f + f FROM ar WHERE id = 4", &["ERR 22003"]),
    ("SELECT id FROM ar WHERE s LIKE 'abc%' ORDER BY id", &["1"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'abc%' ORDER BY id",
        &["2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '%abc' ORDER BY id", &["1"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%abc' ORDER BY id",
        &["2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '%b%' ORDER BY id", &["1"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%b%' ORDER BY id",
        &["2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE 'a_c%' ORDER BY id",
        &["1", "2"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a_c%' ORDER BY id",
        &["3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '%c_' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%c_' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE '%' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s NOT LIKE '%' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s LIKE '_%' ORDER BY id",
        &["1", "2", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '_%' ORDER BY id",
        &["3"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE '%_' ORDER BY id",
        &["1", "2", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%_' ORDER BY id",
        &["3"],
    ),
    ("SELECT id FROM ar WHERE s LIKE 'a%c%d' ORDER BY id", &["2"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a%c%d' ORDER BY id",
        &["1", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE '%a%b%c' ORDER BY id",
        &["1"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%a%b%c' ORDER BY id",
        &["2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE 'abcabc' ORDER BY id",
        &["1"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'abcabc' ORDER BY id",
        &["2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE 'abc' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'abc' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE 'a%%c' ORDER BY id", &["1"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a%%c' ORDER BY id",
        &["2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE '%%' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s NOT LIKE '%%' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s LIKE 'a\\%c_d' ORDER BY id",
        &["2"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a\\%c_d' ORDER BY id",
        &["1", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE 'a\\_c' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a\\_c' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '%\\%%' ORDER BY id", &["2"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%\\%%' ORDER BY id",
        &["1", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '_' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '_' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '__' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '__' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE 'a%b%c%a' ORDER BY id", &[]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a%b%c%a' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE '%abcabc%x' ORDER BY id",
        &[],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%abcabc%x' ORDER BY id",
        &["1", "2", "3", "4"],
    ),
    ("SELECT id FROM ar WHERE s LIKE '%cab%' ORDER BY id", &["1"]),
    (
        "SELECT id FROM ar WHERE s NOT LIKE '%cab%' ORDER BY id",
        &["2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE 'abc_bc' ORDER BY id",
        &["1"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'abc_bc' ORDER BY id",
        &["2", "3", "4"],
    ),
    (
        "SELECT id FROM ar WHERE s LIKE 'a_c_bc' ORDER BY id",
        &["1"],
    ),
    (
        "SELECT id FROM ar WHERE s NOT LIKE 'a_c_bc' ORDER BY id",
        &["2", "3", "4"],
    ),
];

#[test]
fn arithmetic_and_like_answer_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for sql in SETUP {
        ok(&mut f.db, &mut s, sql);
    }
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
