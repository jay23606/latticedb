//! `uuid` text input, output and ordering checked against PostgreSQL 16: the hyphen after any group
//! of four digits, braces, upper case, a missing or extra digit, stray characters and white space
//! (which is not skipped). Each entry lists what PostgreSQL answered (`NULL` is an empty field).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{fixture, text};

const CASES: &[(&str, &[&str])] = &[
    (
        "SELECT '00000000-0000-0000-0000-000000000000'::uuid",
        &["00000000-0000-0000-0000-000000000000"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123E4567-E89B-12D3-A456-426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123e4567e89b12d3a456426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '{123e4567-e89b-12d3-a456-426614174000}'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '{123e4567e89b12d3a456426614174000}'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-42661417400'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-4266141740000'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-42661417400g'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567e89b-12d3-a456-426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123e-4567e89b12d3a456426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456426614174000'::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '-123e4567-e89b-12d3-a456-426614174000'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000-'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '{123e4567-e89b-12d3-a456-426614174000'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000}'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT ' 123e4567-e89b-12d3-a456-426614174000 '::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '1234-5678-9abc-def0-1234-5678-9abc-def0'::uuid",
        &["12345678-9abc-def0-1234-56789abcdef0"],
    ),
    (
        "SELECT '12345678-9abc-def0-1234-56789abcdef0'::uuid",
        &["12345678-9abc-def0-1234-56789abcdef0"],
    ),
    (
        "SELECT '1234 5678-9abc-def0-1234-5678-9abc-def0'::uuid",
        &["ERR 22P02"],
    ),
    ("SELECT ''::uuid", &["ERR 22P02"]),
    (
        "SELECT 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid",
        &["ffffffff-ffff-ffff-ffff-ffffffffffff"],
    ),
    (
        "SELECT 'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF'::uuid",
        &["ffffffff-ffff-ffff-ffff-ffffffffffff"],
    ),
    (
        "SELECT '0000000000000000000000000000000g'::uuid",
        &["ERR 22P02"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000'::uuid = '123E4567-E89B-12D3-A456-426614174000'::uuid",
        &["t"],
    ),
    (
        "SELECT '00000000-0000-0000-0000-000000000001'::uuid < '00000000-0000-0000-0000-000000000002'::uuid",
        &["t"],
    ),
    (
        "SELECT 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid > '00000000-0000-0000-0000-000000000000'::uuid",
        &["t"],
    ),
    (
        "SELECT '80000000-0000-0000-0000-000000000000'::uuid > '7fffffff-ffff-ffff-ffff-ffffffffffff'::uuid",
        &["t"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000'::uuid::text",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000'::text::uuid",
        &["123e4567-e89b-12d3-a456-426614174000"],
    ),
    ("SELECT NULL::uuid", &[""]),
    (
        "SELECT '123e4567-e89b-12d3-a456-426614174000'::uuid IS NULL",
        &["f"],
    ),
];

#[test]
fn uuids_are_read_and_ordered_as_postgresql_does() {
    let mut f = fixture();
    let mut s = common::admin();
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
