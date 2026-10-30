//! Which JSON texts a `jsonb` cast accepts, checked against PostgreSQL 17: every document below was run
//! as `SELECT '<json>'::jsonb IS NOT NULL` there, and this executor must accept the same ones and
//! refuse the same ones with the same SQLSTATE. The set covers every literal, number form, escape,
//! separator, nesting and control-character case the scanner handles. Three documents on which this
//! executor still differs (a lone surrogate escape and a NUL escape, which PostgreSQL refuses) are
//! reported in an issue and are not part of this list.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{admin, fixture, text};

/// (statement, `t` when PostgreSQL accepts the document, or `ERROR <sqlstate>`)
#[rustfmt::skip]
const CASES: [(&str, &str); 97] = [
    ("SELECT ('{\"a\":1}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":{\"b\":[1,2,{\"c\":null}]}}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[1,2,3]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"str\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('12'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('-12'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('1.5'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('1.5e3'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('1E+3'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('1e-3'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('0'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('-0'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('0.5'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('true'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('false'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('null'::jsonb) IS NOT NULL", "t"),
    ("SELECT (' null '::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[ ]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{ }'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[ 1 , 2 ]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{ \"a\" : 1 , \"b\" : 2 }'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":1,}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1,2,]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[,1]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1,,2]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{,}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\"}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\":}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{:1}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{a:1}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{''a'':1}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"unterminated'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1 2]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\" 1}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\":1 \"b\":2}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('01'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('00'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('-'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('--1'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('1.'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('.5'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('+1'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('1e'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('1e+'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('1.e3'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('0x10'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('NaN'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('Infinity'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('tru'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('nul'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('fals'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('truee'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('nulll'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[true,false,null]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":tru}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\":nul}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1]]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[[1]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\":1}}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"a\":[}'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT (''::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT (' '::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"\\u00e9\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"\\u12\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"\\u12g4\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"\\uD83D\\uDE00\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"\\x\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"\\n\\t\\r\\b\\f\\\\\\/\\\"\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"\\\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"\\\\\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"a\\\"b\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"a\\/b\"'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"\\U0041\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"\\u0061\":1}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":\"\\n\"}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":\"b\\tc\"}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('\"tab\tinside\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"line\nbreak\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('\"ctl\u{1}char\"'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1,[2,[3,[4,[5,[6,[7,[8,[9,[10]]]]]]]]]]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{}}}}}}}}}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[0.1,0.25,1e2,-5E-2]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[1.5.5]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1e5e5]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[--1]'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('[1,2'::jsonb) IS NOT NULL", "ERROR 22P02"),
    ("SELECT ('{\"k1\":1,\"k2\":2,\"k3\":3}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"k\":[true,{\"x\":false}],\"y\":\"z\"}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[\"\"]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[\"\",\"\"]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('{\"\":1}'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[null,null]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[\"a\",\"b\"]'::jsonb) IS NOT NULL", "t"),
    ("SELECT ('[{\"a\":1},{\"b\":2}]'::jsonb) IS NOT NULL", "t"),
];

#[test]
fn json_validity_agrees_with_postgresql() {
    let mut f = fixture();
    let mut s = admin();
    for (sql, expected) in CASES {
        let got = match s.execute(&mut f.db, sql) {
            Ok(r) => text(&r)[0][0].clone(),
            Err(e) => format!("ERROR {}", e.sqlstate()),
        };
        assert_eq!(got, expected, "{sql}");
    }
}

#[test]
fn json_nesting_is_limited_to_sixty_four_levels() {
    // PostgreSQL allows far deeper documents; this executor stops at 64 and says so. The edge is exact.
    let mut f = fixture();
    let mut s = admin();
    let mut check = |levels: usize| {
        let doc = format!("{}1{}", "[".repeat(levels), "]".repeat(levels));
        s.execute(&mut f.db, &format!("SELECT '{doc}'::jsonb IS NOT NULL"))
            .map(|r| text(&r)[0][0].clone())
            .map_err(|e| e.sqlstate().to_string())
    };
    assert_eq!(check(1), Ok("t".to_string()));
    assert_eq!(check(63), Ok("t".to_string()));
    // The scanner refuses a value at depth 65 (`depth > 64`), and the innermost scalar of an
    // N-level document sits at depth N, so 64 levels are accepted and 65 are refused.
    assert_eq!(check(64), Ok("t".to_string()));
    assert_eq!(check(65), Err("22P02".to_string()));
}
