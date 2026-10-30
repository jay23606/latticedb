//! Identifier case, quoting and keywords used as names, in one session, checked against PostgreSQL 16:
//! unquoted names fold to lower case, quoted names keep their case, reserved words are refused as
//! bare names but allowed after `AS` as a column label. Each step lists what PostgreSQL answered:
//! rows then, for statements that are not queries, the command tag, or `ERR` and the SQLSTATE.
//! Identifiers longer than 63 bytes are left out: PostgreSQL truncates them, this server refuses
//! them.
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
        "CREATE TABLE Mixed (ID INT PRIMARY KEY, Name TEXT)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO mixed VALUES (1, 'a')", &["INSERT 0 1"]),
    (
        "INSERT INTO MIXED (id, NAME) VALUES (2, 'b')",
        &["INSERT 0 1"],
    ),
    ("SELECT id, name FROM MiXeD ORDER BY ID", &["1|a", "2|b"]),
    (
        "SELECT \"id\", \"name\" FROM \"mixed\" ORDER BY 1",
        &["1|a", "2|b"],
    ),
    ("SELECT \"ID\" FROM mixed", &["ERR 42703"]),
    ("SELECT \"Mixed\".id FROM \"Mixed\"", &["ERR 42P01"]),
    ("SELECT mixed.id FROM mixed ORDER BY mixed.id", &["1", "2"]),
    ("SELECT m.id FROM mixed AS m ORDER BY m.id", &["1", "2"]),
    ("SELECT m.id FROM mixed m ORDER BY m.id", &["1", "2"]),
    ("SELECT mixed.id FROM mixed m", &["ERR 42P01"]),
    (
        "SELECT id AS \"My Col\", name n FROM mixed ORDER BY id",
        &["1|a", "2|b"],
    ),
    ("SELECT id AS Upper FROM mixed ORDER BY id LIMIT 1", &["1"]),
    (
        "CREATE TABLE \"Quoted\" (\"Col A\" INT PRIMARY KEY, \"select\" TEXT)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO \"Quoted\" VALUES (1, 'x')", &["INSERT 0 1"]),
    ("SELECT \"Col A\", \"select\" FROM \"Quoted\"", &["1|x"]),
    ("SELECT col a FROM \"Quoted\"", &["ERR 42703"]),
    ("SELECT * FROM quoted", &["ERR 42P01"]),
    ("SELECT * FROM Quoted", &["ERR 42P01"]),
    ("CREATE TABLE \"a\"\"b\" (x INT)", &["CREATE TABLE"]),
    ("INSERT INTO \"a\"\"b\" VALUES (1)", &["INSERT 0 1"]),
    ("SELECT x FROM \"a\"\"b\"", &["1"]),
    (
        "CREATE TABLE t2 (id INT, id2 INT, PRIMARY KEY (id))",
        &["CREATE TABLE"],
    ),
    ("CREATE TABLE select (a INT)", &["ERR 42601"]),
    ("CREATE TABLE \"\" (a INT)", &["ERR 42601"]),
    ("CREATE TABLE t3 (a INT, \"A\" INT)", &["CREATE TABLE"]),
    ("CREATE TABLE t4 (a INT, A INT)", &["ERR 42701"]),
    ("CREATE TABLE t5 (user INT)", &["ERR 42601"]),
    (
        "CREATE TABLE t6 (name INT, type INT, key INT, value INT, data INT, text INT)",
        &["CREATE TABLE"],
    ),
    ("INSERT INTO t6 VALUES (1, 2, 3, 4, 5, 6)", &["INSERT 0 1"]),
    (
        "SELECT name, type, key, value, data, text FROM t6",
        &["1|2|3|4|5|6"],
    ),
    ("CREATE TABLE t7 (end INT)", &["ERR 42601"]),
    ("CREATE TABLE t8 (table INT)", &["ERR 42601"]),
    ("CREATE TABLE t9 (order INT)", &["ERR 42601"]),
    ("CREATE TABLE t10 (limit INT)", &["ERR 42601"]),
    ("CREATE TABLE t11 (offset INT)", &["ERR 42601"]),
    ("CREATE TABLE t12 (all INT)", &["ERR 42601"]),
    ("CREATE TABLE t13 (default INT)", &["ERR 42601"]),
    ("CREATE TABLE t14 (primary INT)", &["ERR 42601"]),
    ("SELECT 1 AS select", &["1"]),
    ("SELECT 1 AS \"select\"", &["1"]),
    ("SELECT 1 AS limit", &["1"]),
    ("SELECT 1 AS name", &["1"]),
    ("SELECT 1 name", &["1"]),
    ("SELECT 1 AS from", &["1"]),
    ("SELECT 1 AS as", &["1"]),
    ("SELECT 1 x y", &["ERR 42601"]),
    ("SELECT 'a' AS \"A\", 'b' AS \"a\"", &["a|b"]),
    ("SELECT 1 AS \"x y\", 2 AS \"X\"", &["1|2"]),
    ("SELECT 1 AS _under, 2 AS d$ollar, 3 AS é", &["1|2|3"]),
    ("SELECT 1 AS 1x", &["ERR 42601"]),
    ("SELECT 1 AS \"ab\"", &["1"]),
    ("SELECT 2 AS a1", &["2"]),
    ("CREATE TABLE ünï (é INT)", &["CREATE TABLE"]),
    ("INSERT INTO ünï VALUES (1)", &["INSERT 0 1"]),
    ("SELECT é FROM ünï", &["1"]),
    ("SELECT É FROM ünï", &["ERR 42703"]),
    ("SELECT \"É\" FROM ünï", &["ERR 42703"]),
];

#[test]
fn identifiers_resolve_as_postgresql_resolves_them() {
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
