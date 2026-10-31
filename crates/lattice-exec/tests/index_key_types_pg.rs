//! Index scans over each key type the planner narrows on, checked against PostgreSQL.
//!
//! Every table has one key, of one type, and an `INT` column `v` that lets the full-scan oracle be
//! built (see `forced`). A predicate on the key runs twice: as written, where the planner may narrow the
//! read to a point or a key range, and with a conjunct that makes it read the whole table. The rows,
//! their order and any error must agree, and the access path must be the one `access.rs` promises.
//!
//! The expected rows are PostgreSQL's answers, written from PostgreSQL's ordering rules before this
//! file was run, not taken from this executor. The values are chosen so that the answer does not depend
//! on the collation (text is lowercase ASCII) or on the time zone (timestamps print in UTC). NUMERIC keys
//! are refused by `CREATE TABLE`, so that type is pinned as refused, and its columns are pinned as never
//! narrowing a scan.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{Db, admin, err, fixture, ok, rows};
use lattice_exec::{AccessPath, ExecSession};

type Outcome = Result<Vec<Vec<String>>, String>;

/// Each case: a predicate on the key column, the rows PostgreSQL returns for it in key order, and the
/// access path the planner must choose.
type Cases<'a> = &'a [(&'a str, &'a [&'a str], AccessPath)];

fn run(db: &mut Db, s: &mut ExecSession, sql: &str) -> Outcome {
    s.execute(db, sql)
        .map(|r| common::text(&r))
        .map_err(|e| e.sqlstate().to_string())
}

/// `pred` and a conjunct that is always true or null where `pred` is, but that the planner cannot
/// show to be error free, so the scan reads the whole table. `v` is an `INT` in every table here.
fn forced(pred: &str) -> String {
    format!("({pred}) AND (CAST(v AS BIGINT) = v OR v IS NULL)")
}

/// Runs `SELECT * FROM table WHERE pred` as written and forced, requires equal outcomes, and returns
/// the access path the written predicate took.
fn same(db: &mut Db, s: &mut ExecSession, table: &str, pred: &str) -> AccessPath {
    let narrow = run(db, s, &format!("SELECT * FROM {table} WHERE {pred}"));
    let path = db.last_access_paths();
    let full = run(
        db,
        s,
        &format!("SELECT * FROM {table} WHERE {}", forced(pred)),
    );
    assert_eq!(
        db.last_access_paths(),
        vec![AccessPath::FullScan],
        "the oracle must read the whole table: {table} WHERE {pred}"
    );
    assert_eq!(
        narrow, full,
        "index scan differs from full scan: {table} WHERE {pred}"
    );
    path.first().copied().unwrap_or(AccessPath::FullScan)
}

/// Creates `table` with `ddl`, loads `values`, then checks every case: the rows PostgreSQL gives for
/// `SELECT key FROM table WHERE pred ORDER BY key`, the access path, and agreement with a full scan.
fn check(table: &str, key: &str, ddl: &str, values: &str, cases: Cases<'_>) {
    let mut f = fixture();
    let mut s = admin();
    ok(&mut f.db, &mut s, ddl);
    ok(
        &mut f.db,
        &mut s,
        &format!("INSERT INTO {table} VALUES {values}"),
    );
    for &(pred, want, path) in cases {
        let got = rows(
            &mut f.db,
            &mut s,
            &format!("SELECT {key} FROM {table} WHERE {pred} ORDER BY {key}"),
        );
        let want: Vec<Vec<String>> = want.iter().map(|w| vec![(*w).to_string()]).collect();
        assert_eq!(got, want, "{table}: {pred}");
        assert_eq!(
            same(&mut f.db, &mut s, table, pred),
            path,
            "{table}: {pred}"
        );
    }
}

#[test]
fn smallint_keys_match_postgresql() {
    check(
        "t_smallint",
        "k",
        "CREATE TABLE t_smallint (k SMALLINT PRIMARY KEY, v INT)",
        "(-32768, 0), (-1, 1), (0, 2), (1, 3), (32767, 4)",
        &[
            ("k = -1", &["-1"], AccessPath::PrimaryKeyPoint),
            ("k = 2", &[], AccessPath::PrimaryKeyPoint),
            ("k > -1", &["0", "1", "32767"], AccessPath::PrimaryKeyRange),
            (
                "k <= 0",
                &["-32768", "-1", "0"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k >= -1 AND k < 1",
                &["-1", "0"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN -1 AND 32767",
                &["-1", "0", "1", "32767"],
                AccessPath::PrimaryKeyRange,
            ),
            ("k < -32768", &[], AccessPath::PrimaryKeyRange),
        ],
    );
}

#[test]
fn bigint_keys_match_postgresql() {
    check(
        "t_bigint",
        "k",
        "CREATE TABLE t_bigint (k BIGINT PRIMARY KEY, v INT)",
        "(-9223372036854775808, 0), (-1, 1), (0, 2), (1, 3), (9223372036854775807, 4)",
        &[
            // The literal 9223372036854775808 does not fit in a BIGINT, so the negated value is not a
            // `Datum::Int` and `key_compatible` in access.rs refuses it: the planner reads the whole
            // table. The row returned is still checked against PostgreSQL's answer.
            (
                "k = -9223372036854775808",
                &["-9223372036854775808"],
                AccessPath::FullScan,
            ),
            ("k = 0", &["0"], AccessPath::PrimaryKeyPoint),
            (
                "k > -1",
                &["0", "1", "9223372036854775807"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k <= 0",
                &["-9223372036854775808", "-1", "0"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k >= -1 AND k < 1",
                &["-1", "0"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN 0 AND 9223372036854775807",
                &["0", "1", "9223372036854775807"],
                AccessPath::PrimaryKeyRange,
            ),
            ("k > 9223372036854775807", &[], AccessPath::PrimaryKeyRange),
        ],
    );
}

#[test]
fn text_keys_match_postgresql() {
    // Shorter prefixes sort first, and every value is lowercase ASCII, so the collation does not matter.
    check(
        "t_text",
        "k",
        "CREATE TABLE t_text (k TEXT PRIMARY KEY, v INT)",
        "('', 0), ('a', 1), ('aa', 2), ('ab', 3), ('b', 4)",
        &[
            ("k = 'a'", &["a"], AccessPath::PrimaryKeyPoint),
            ("k = ''", &[""], AccessPath::PrimaryKeyPoint),
            ("k > 'a'", &["aa", "ab", "b"], AccessPath::PrimaryKeyRange),
            ("k < 'ab'", &["", "a", "aa"], AccessPath::PrimaryKeyRange),
            (
                "k >= 'a' AND k <= 'ab'",
                &["a", "aa", "ab"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN 'aa' AND 'b'",
                &["aa", "ab", "b"],
                AccessPath::PrimaryKeyRange,
            ),
            // Two points joined by OR do not narrow: the whole table is read.
            ("k = 'aa' OR k = 'b'", &["aa", "b"], AccessPath::FullScan),
        ],
    );
}

#[test]
fn bytea_keys_match_postgresql() {
    // Byte order, with a shorter value first when one is a prefix of the other. The zero byte is the
    // case the key encoding escapes, so `\x00` and `\x0001` are both in play.
    check(
        "t_bytea",
        "k",
        "CREATE TABLE t_bytea (k BYTEA PRIMARY KEY, v INT)",
        "('\\x'::bytea, 0), ('\\x00'::bytea, 1), ('\\x0001'::bytea, 2), ('\\x01'::bytea, 3), ('\\xff'::bytea, 4)",
        &[
            (
                "k = '\\x00'::bytea",
                &["\\x00"],
                AccessPath::PrimaryKeyPoint,
            ),
            ("k = '\\x'::bytea", &["\\x"], AccessPath::PrimaryKeyPoint),
            (
                "k > '\\x00'::bytea",
                &["\\x0001", "\\x01", "\\xff"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k < '\\x01'::bytea",
                &["\\x", "\\x00", "\\x0001"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN '\\x00'::bytea AND '\\x0001'::bytea",
                &["\\x00", "\\x0001"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k >= '\\x0001'::bytea AND k < '\\xff'::bytea",
                &["\\x0001", "\\x01"],
                AccessPath::PrimaryKeyRange,
            ),
        ],
    );
}

#[test]
fn date_keys_match_postgresql() {
    // Dates before 1970 are negative day numbers, so their key bytes must sort before 1970's.
    check(
        "t_date",
        "k",
        "CREATE TABLE t_date (k DATE PRIMARY KEY, v INT)",
        "('1900-01-01'::date, 0), ('1969-12-31'::date, 1), ('1970-01-01'::date, 2), ('2000-02-29'::date, 3), ('9999-12-31'::date, 4)",
        &[
            (
                "k = '1969-12-31'::date",
                &["1969-12-31"],
                AccessPath::PrimaryKeyPoint,
            ),
            (
                "k < '1970-01-01'::date",
                &["1900-01-01", "1969-12-31"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k >= '1969-12-31'::date AND k <= '2000-02-29'::date",
                &["1969-12-31", "1970-01-01", "2000-02-29"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k > '2000-02-29'::date",
                &["9999-12-31"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN '1900-01-01'::date AND '1969-12-31'::date",
                &["1900-01-01", "1969-12-31"],
                AccessPath::PrimaryKeyRange,
            ),
        ],
    );
}

#[test]
fn timestamptz_keys_match_postgresql() {
    // PostgreSQL prints in UTC here. The row at 10:00:00.5 sits half a second after a whole second, so
    // the bounds that fall inside that gap are exact. The pre-epoch row checks the sign of the encoded
    // instant.
    check(
        "t_timestamptz",
        "k",
        "CREATE TABLE t_timestamptz (k TIMESTAMPTZ PRIMARY KEY, v INT)",
        "('1969-12-31 23:59:59+00'::timestamptz, 0), ('2000-01-01 00:00:00+00'::timestamptz, 1), ('2024-06-01 12:00:00+02'::timestamptz, 2), ('2024-06-01 10:00:00.5+00'::timestamptz, 3), ('9999-12-31 23:59:59+00'::timestamptz, 4)",
        &[
            (
                "k = '2024-06-01 12:00:00+02'::timestamptz",
                &["2024-06-01 10:00:00+00"],
                AccessPath::PrimaryKeyPoint,
            ),
            (
                "k > '2000-01-01 00:00:00+00'::timestamptz",
                &[
                    "2024-06-01 10:00:00+00",
                    "2024-06-01 10:00:00.5+00",
                    "9999-12-31 23:59:59+00",
                ],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k < '2000-01-01 00:00:00+00'::timestamptz",
                &["1969-12-31 23:59:59+00"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k >= '2024-06-01 10:00:00+00'::timestamptz AND k < '2024-06-01 10:00:00.5+00'::timestamptz",
                &["2024-06-01 10:00:00+00"],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN '1969-12-31 23:59:59+00'::timestamptz AND '2000-01-01 00:00:00+00'::timestamptz",
                &["1969-12-31 23:59:59+00", "2000-01-01 00:00:00+00"],
                AccessPath::PrimaryKeyRange,
            ),
        ],
    );
}

#[test]
fn uuid_keys_match_postgresql() {
    // A UUID is 16 unsigned bytes: the values from 0x80 up must sort after the ones below 0x80.
    check(
        "t_uuid",
        "k",
        "CREATE TABLE t_uuid (k UUID PRIMARY KEY, v INT)",
        "('00000000-0000-0000-0000-000000000000'::uuid, 0), ('00000000-0000-0000-0000-000000000001'::uuid, 1), ('7fffffff-ffff-ffff-ffff-ffffffffffff'::uuid, 2), ('80000000-0000-0000-0000-000000000000'::uuid, 3), ('ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid, 4)",
        &[
            (
                "k = 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid",
                &["ffffffff-ffff-ffff-ffff-ffffffffffff"],
                AccessPath::PrimaryKeyPoint,
            ),
            (
                "k > '7fffffff-ffff-ffff-ffff-ffffffffffff'::uuid",
                &[
                    "80000000-0000-0000-0000-000000000000",
                    "ffffffff-ffff-ffff-ffff-ffffffffffff",
                ],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k <= '80000000-0000-0000-0000-000000000000'::uuid",
                &[
                    "00000000-0000-0000-0000-000000000000",
                    "00000000-0000-0000-0000-000000000001",
                    "7fffffff-ffff-ffff-ffff-ffffffffffff",
                    "80000000-0000-0000-0000-000000000000",
                ],
                AccessPath::PrimaryKeyRange,
            ),
            (
                "k BETWEEN '00000000-0000-0000-0000-000000000001'::uuid AND '80000000-0000-0000-0000-000000000000'::uuid",
                &[
                    "00000000-0000-0000-0000-000000000001",
                    "7fffffff-ffff-ffff-ffff-ffffffffffff",
                    "80000000-0000-0000-0000-000000000000",
                ],
                AccessPath::PrimaryKeyRange,
            ),
        ],
    );
}

#[test]
fn boolean_keys_match_postgresql() {
    check(
        "t_boolean",
        "k",
        "CREATE TABLE t_boolean (k BOOLEAN PRIMARY KEY, v INT)",
        "(false, 0), (true, 1)",
        &[
            ("k = true", &["t"], AccessPath::PrimaryKeyPoint),
            ("k = false", &["f"], AccessPath::PrimaryKeyPoint),
            ("k > false", &["t"], AccessPath::PrimaryKeyRange),
            ("k < true", &["f"], AccessPath::PrimaryKeyRange),
            (
                "k >= false AND k <= true",
                &["f", "t"],
                AccessPath::PrimaryKeyRange,
            ),
        ],
    );
}

#[test]
fn numeric_keys_are_refused_and_numeric_columns_never_narrow() {
    // The executor cannot encode a numeric key, so `CREATE TABLE` refuses one.
    let mut f = fixture();
    let mut s = admin();
    let e = err(
        &mut f.db,
        &mut s,
        "CREATE TABLE t_numeric_key (k NUMERIC PRIMARY KEY, v INT)",
    );
    assert!(e.to_string().to_lowercase().contains("numeric"), "{e}");

    // A numeric column is never a key, so a table with a BIGINT key still reads every row for a
    // predicate on the numeric column alone: every path below is a full scan.
    check(
        "t_numeric",
        "x",
        "CREATE TABLE t_numeric (id BIGINT PRIMARY KEY, x NUMERIC, v INT)",
        "(1, -2.5, 0), (2, 0.1, 1), (3, 1.25, 2), (4, 10, 3)",
        &[
            ("x = 1.25", &["1.25"], AccessPath::FullScan),
            ("x > 0.1", &["1.25", "10"], AccessPath::FullScan),
            ("x <= 0.1", &["-2.5", "0.1"], AccessPath::FullScan),
            (
                "x BETWEEN 0 AND 1.25",
                &["0.1", "1.25"],
                AccessPath::FullScan,
            ),
            ("x = 10", &["10"], AccessPath::FullScan),
        ],
    );
}
