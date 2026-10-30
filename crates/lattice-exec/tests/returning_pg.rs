//! `INSERT`, `UPDATE` and `DELETE` with `RETURNING` in one session, checked against PostgreSQL 16:
//! returned rows then the command tag, defaults, expressions, aliases, constraint errors, and a
//! `RETURNING` expression that fails. Each step lists what PostgreSQL answered. The last two
//! unordered statements of the original run are left out, because PostgreSQL returns those rows in
//! heap order.
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
        "CREATE TABLE r (id INT PRIMARY KEY, n INT NOT NULL DEFAULT 7, s TEXT, u INT UNIQUE)",
        &["CREATE TABLE"],
    ),
    (
        "INSERT INTO r VALUES (1, 1, 'a', 10) RETURNING *",
        &["1|1|a|10", "INSERT 0 1"],
    ),
    (
        "INSERT INTO r VALUES (2, 2, 'b', 11), (3, 3, NULL, 12) RETURNING id, s",
        &["2|b", "3|", "INSERT 0 2"],
    ),
    (
        "INSERT INTO r (id) VALUES (4) RETURNING id, n, s, u",
        &["4|7||", "INSERT 0 1"],
    ),
    (
        "INSERT INTO r (id) VALUES (5) RETURNING n + 1, s IS NULL, 'k'",
        &["8|t|k", "INSERT 0 1"],
    ),
    (
        "INSERT INTO r (id) VALUES (6) RETURNING id AS the_id",
        &["6", "INSERT 0 1"],
    ),
    (
        "INSERT INTO r VALUES (1, 1, 'a', 99) RETURNING *",
        &["ERR 23505"],
    ),
    (
        "INSERT INTO r VALUES (7, 7, 'g', 10) RETURNING id",
        &["ERR 23505"],
    ),
    ("INSERT INTO r VALUES (8) RETURNING zzz", &["ERR 42703"]),
    (
        "INSERT INTO r (id) VALUES (9) RETURNING *, id",
        &["9|7|||9", "INSERT 0 1"],
    ),
    (
        "UPDATE r SET n = n + 10 WHERE id <= 2 RETURNING id, n",
        &["1|11", "2|12", "UPDATE 2"],
    ),
    (
        "UPDATE r SET s = 'z' WHERE id = 100 RETURNING *",
        &["UPDATE 0"],
    ),
    (
        "UPDATE r SET n = NULL WHERE id = 1 RETURNING *",
        &["ERR 23502"],
    ),
    (
        "UPDATE r SET s = s || '!' WHERE id = 2 RETURNING s, length(s)",
        &["b!|2", "UPDATE 1"],
    ),
    (
        "UPDATE r SET id = id + 50 WHERE id = 3 RETURNING id",
        &["53", "UPDATE 1"],
    ),
    (
        "UPDATE r SET u = 10 WHERE id = 2 RETURNING *",
        &["ERR 23505"],
    ),
    (
        "DELETE FROM r WHERE id = 53 RETURNING *",
        &["53|3||12", "DELETE 1"],
    ),
    ("DELETE FROM r WHERE id = 53 RETURNING *", &["DELETE 0"]),
    (
        "DELETE FROM r WHERE id IN (4, 5) RETURNING id, n",
        &["4|7", "5|7", "DELETE 2"],
    ),
    (
        "DELETE FROM r WHERE id = 6 RETURNING id + 1000",
        &["1006", "DELETE 1"],
    ),
    (
        "SELECT id, n, s, u FROM r ORDER BY id",
        &["1|11|a|10", "2|12|b!|11", "9|7||"],
    ),
    (
        "INSERT INTO r VALUES (20, 1, 'q', 30) RETURNING 1 / 0",
        &["ERR 22012"],
    ),
    ("SELECT id FROM r ORDER BY id", &["1", "2", "9"]),
    (
        "INSERT INTO r (id, s) VALUES (21, 'tx') RETURNING lower(s), upper(s)",
        &["tx|TX", "INSERT 0 1"],
    ),
];

#[test]
fn returning_answers_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in STEPS {
        let got: Vec<String> = match s.execute(&mut f.db, sql) {
            Ok(r) => {
                let mut lines: Vec<String> = text(&r)
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|c| if c == "NULL" { "" } else { c.as_str() })
                            .collect::<Vec<_>>()
                            .join("|")
                    })
                    .collect();
                if !sql.starts_with("SELECT") {
                    lines.push(r.tag);
                }
                lines
            }
            Err(e) => vec![format!("ERR {}", e.sqlstate())],
        };
        assert_eq!(&got, want, "{sql}");
    }
}
