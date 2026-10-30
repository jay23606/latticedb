//! String functions, `LIKE` and text comparison checked against PostgreSQL 16. Every statement and
//! expected value below was produced by PostgreSQL; locale-dependent case mapping and collation of
//! non-ASCII text is left out on purpose.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{err, fixture, rows};

const VALUES: &[(&str, &str)] = &[
    ("SELECT substring('hello' from 2 for 3)", "ell"),
    ("SELECT substring('hello' from 0 for 3)", "he"),
    ("SELECT substring('hello' from -1 for 4)", "he"),
    ("SELECT substring('hello' from 4)", "lo"),
    ("SELECT substring('hello' from 9)", ""),
    ("SELECT substring('hello' from 2 for 0)", ""),
    ("SELECT substring('héllo' from 2 for 2)", "él"),
    ("SELECT substring(NULL from 1)", "NULL"),
    ("SELECT substring('hello' from NULL)", "NULL"),
    ("SELECT position('l' in 'hello')", "3"),
    ("SELECT position('' in 'hello')", "1"),
    ("SELECT position('lo' in 'héllo')", "4"),
    ("SELECT position('z' in 'hello')", "0"),
    ("SELECT position(NULL in 'a')", "NULL"),
    ("SELECT length('héllo')", "5"),
    ("SELECT length('')", "0"),
    ("SELECT length(NULL::text)", "NULL"),
    ("SELECT trim(both 'x' from 'xxaxx')", "a"),
    ("SELECT trim(leading 'xy' from 'xyyxaxy')", "axy"),
    ("SELECT trim(trailing 'xy' from 'xyyxaxy')", "xyyxa"),
    ("SELECT trim('  a  ')", "a"),
    ("SELECT trim(both '' from 'abc')", "abc"),
    ("SELECT trim(both NULL from 'abc')", "NULL"),
    ("SELECT 'a' || 'b' || NULL", "NULL"),
    ("SELECT 'abc' LIKE 'a%'", "t"),
    ("SELECT 'abc' LIKE 'a_c'", "t"),
    ("SELECT 'abc' LIKE 'A%'", "f"),
    ("SELECT 'a%c' LIKE 'a\\%c'", "t"),
    ("SELECT 'abc' LIKE ''", "f"),
    ("SELECT '' LIKE ''", "t"),
    ("SELECT 'abc' NOT LIKE 'a%'", "f"),
    ("SELECT NULL LIKE 'a'", "NULL"),
    ("SELECT 'abc' < 'abd'", "t"),
    ("SELECT 'abc' = 'ABC'", "f"),
];

const ERRORS: &[(&str, &str)] = &[
    ("SELECT substring('hello' from 2 for -1)", "22011"),
    ("SELECT 'a\\' LIKE 'a\\'", "22025"),
];

#[test]
fn string_statements_return_what_postgresql_returns() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in VALUES {
        let got = rows(&mut f.db, &mut s, sql);
        assert_eq!(
            got.first().and_then(|r| r.first()).map(String::as_str),
            Some(*want),
            "{sql}"
        );
    }
}

#[test]
fn string_statements_fail_with_postgresqls_sqlstate() {
    let mut f = fixture();
    let mut s = common::admin();
    for (sql, want) in ERRORS {
        assert_eq!(err(&mut f.db, &mut s, sql).sqlstate(), *want, "{sql}");
    }
}
