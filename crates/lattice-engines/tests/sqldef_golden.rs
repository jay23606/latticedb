//! The SQL definition codec against the independent fixtures of
//! `formats/generate_sql_definition_golden.py` (RFC 0012). Expected outcomes below are literals
//! written from the format document, never produced by running the encoder.
#![cfg(feature = "row")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_core::bugs::{self, Bug};
use lattice_engines::sqldef::{
    self, Column, DefaultValue, DefineTable, KeyConstraint, SqlDefError, SqlTableDef, SqlType,
};

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            "../../../formats/corpus/sql-definition-v1-",
            $name,
            ".golden"
        ))
        .as_slice()
    };
}

struct BugGuard;
impl Drop for BugGuard {
    fn drop(&mut self) {
        bugs::clear();
    }
}

fn column(name: &str, ty: SqlType) -> Column {
    Column {
        name: name.into(),
        ty,
        varchar_limit: 0,
        not_null: false,
        default: None,
    }
}

fn key(columns: &[u16], name: &str) -> KeyConstraint {
    KeyConstraint {
        columns: columns.to_vec(),
        name: name.into(),
    }
}

fn composite() -> SqlTableDef {
    let mut tenant = column("tenant", SqlType::Uuid);
    tenant.not_null = true;
    let mut id = column("id", SqlType::BigInt);
    id.not_null = true;
    SqlTableDef {
        columns: vec![tenant, id, column("title", SqlType::Text)],
        primary_key: Some(key(&[0, 1], "items_pkey")),
        uniques: vec![key(&[2], "items_title_key")],
    }
}

fn minimal() -> SqlTableDef {
    SqlTableDef {
        columns: vec![column("id", SqlType::Integer)],
        primary_key: None,
        uniques: Vec::new(),
    }
}

#[test]
fn the_minimal_definition_has_the_literal_bytes_and_value() {
    let bytes = golden!("ok-minimal");
    // version 1, layout 1, one column "id", integer, no limit, flags 0, no key, no uniques.
    assert_eq!(
        bytes,
        [
            1, 1, 1, 0, 2, 0, 0, 0, b'i', b'd', 3, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(SqlTableDef::decode(bytes).unwrap(), minimal());
    assert_eq!(minimal().encode().unwrap(), bytes);
}

#[test]
fn the_composite_key_definition_decodes_to_its_literal_value() {
    let bytes = golden!("ok-composite-key");
    assert_eq!(SqlTableDef::decode(bytes).unwrap(), composite());
    assert_eq!(composite().encode().unwrap(), bytes);
}

#[test]
fn every_type_tag_decodes_to_its_type() {
    let def = SqlTableDef::decode(golden!("ok-all-types")).unwrap();
    let types = [
        SqlType::Boolean,
        SqlType::SmallInt,
        SqlType::Integer,
        SqlType::BigInt,
        SqlType::Double,
        SqlType::Numeric,
        SqlType::Text,
        SqlType::Varchar,
        SqlType::Bytea,
        SqlType::Uuid,
        SqlType::Date,
        SqlType::Timestamptz,
        SqlType::Jsonb,
    ];
    assert_eq!(def.columns.len(), 13);
    for (i, (col, ty)) in def.columns.iter().zip(types).enumerate() {
        assert_eq!(col.name, format!("c{}", i + 1));
        assert_eq!(col.ty, ty);
        assert_eq!(usize::from(ty.tag()), i + 1);
        assert_eq!(
            col.varchar_limit,
            if ty == SqlType::Varchar { 10 } else { 0 }
        );
    }
    assert_eq!(def.encode().unwrap(), golden!("ok-all-types"));
}

#[test]
fn every_default_tag_decodes_to_its_literal_value() {
    let def = SqlTableDef::decode(golden!("ok-all-defaults")).unwrap();
    let defaults: Vec<DefaultValue> = def
        .columns
        .iter()
        .map(|c| c.default.clone().unwrap())
        .collect();
    assert_eq!(
        defaults,
        vec![
            DefaultValue::Bool(true),
            DefaultValue::Int(-32768),
            DefaultValue::Int(2_147_483_647),
            DefaultValue::Int(i64::MIN),
            DefaultValue::Double((-1.5f64).to_bits()),
            DefaultValue::Double(0x7ff8_0000_0000_0000),
            DefaultValue::Numeric("-12.034".into()),
            DefaultValue::Text("h\u{e9}llo".into()),
            DefaultValue::Text("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}".into()),
            DefaultValue::Bytes(vec![0, 255]),
            DefaultValue::Uuid(std::array::from_fn(|i| i as u8)),
            DefaultValue::Date(-1),
            DefaultValue::Timestamptz(1_700_000_000_000_000),
            DefaultValue::Jsonb("{\"b\": 1,  \"a\": [true]}".into()),
        ]
    );
    // The varchar limit counts characters: five two-byte characters fit a limit of five.
    assert_eq!(def.columns[8].varchar_limit, 5);
    assert_eq!(def.encode().unwrap(), golden!("ok-all-defaults"));
}

#[test]
fn fifteen_uniques_are_accepted_in_order() {
    let def = SqlTableDef::decode(golden!("ok-fifteen-uniques")).unwrap();
    assert_eq!(def.uniques.len(), 15);
    for (k, unique) in def.uniques.iter().enumerate() {
        assert_eq!(unique.columns, vec![k as u16]);
        assert_eq!(unique.name, format!("u{k}"));
    }
    assert_eq!(def.encode().unwrap(), golden!("ok-fifteen-uniques"));
}

#[test]
fn the_accepted_definition_blocks_decode_and_re_encode() {
    let two = sqldef::decode_ops(golden!("ops-ok-two")).unwrap();
    assert_eq!(
        two,
        vec![
            DefineTable {
                table_id: 5,
                definition: minimal()
            },
            DefineTable {
                table_id: 9,
                definition: composite()
            },
        ]
    );
    assert_eq!(sqldef::encode_ops(&two).unwrap(), golden!("ops-ok-two"));
    let max = sqldef::decode_ops(golden!("ops-ok-max-table-id")).unwrap();
    assert_eq!(max[0].table_id, 67_108_863);
    assert_eq!(
        sqldef::encode_ops(&max).unwrap(),
        golden!("ops-ok-max-table-id")
    );
    // A containing envelope reads the fields after the block: the prefix reader reports its length.
    let mut with_tail = golden!("ops-ok-two").to_vec();
    with_tail.extend_from_slice(b"rows");
    let (ops, used) = sqldef::decode_ops_prefix(&with_tail).unwrap();
    assert_eq!((ops.len(), used), (2, with_tail.len() - 4));
}

macro_rules! rejects {
    ($decode:path; $($name:literal => $err:expr),+ $(,)?) => {
        vec![$(($name, golden!($name), $err, $decode(golden!($name)).err())),+]
    };
}

#[test]
fn every_rejection_fixture_is_refused_for_its_rule() {
    let cases = rejects!(SqlTableDef::decode;
        "reject-version" => SqlDefError::UnsupportedVersion(2),
        "reject-layout-rule" => SqlDefError::UnsupportedLayout(2),
        "reject-zero-columns" => SqlDefError::BadColumnCount,
        "reject-type-tag-zero" => SqlDefError::BadTypeTag(0),
        "reject-type-tag-14" => SqlDefError::BadTypeTag(14),
        "reject-flag-bit-2" => SqlDefError::BadFlags(4),
        "reject-flag-bit-7" => SqlDefError::BadFlags(0x80),
        "reject-uppercase-name" => SqlDefError::BadName,
        "reject-empty-name" => SqlDefError::BadName,
        "reject-name-256-bytes" => SqlDefError::BadName,
        "reject-name-invalid-utf8" => SqlDefError::BadUtf8,
        "reject-duplicate-column" => SqlDefError::DuplicateColumn,
        "reject-varchar-limit-on-text" => SqlDefError::BadVarcharLimit,
        "reject-default-tag-11" => SqlDefError::BadDefaultTag(11),
        "reject-default-mismatch" => SqlDefError::DefaultMismatch,
        "reject-default-smallint-range" => SqlDefError::DefaultOutOfRange,
        "reject-default-integer-range" => SqlDefError::DefaultOutOfRange,
        "reject-default-numeric-trailing-zero" => SqlDefError::BadNumeric,
        "reject-default-numeric-leading-zero" => SqlDefError::BadNumeric,
        "reject-default-numeric-negative-zero" => SqlDefError::BadNumeric,
        "reject-default-numeric-text" => SqlDefError::BadNumeric,
        "reject-default-nan-noncanonical" => SqlDefError::DefaultOutOfRange,
        "reject-default-bool-byte-2" => SqlDefError::BadBool,
        "reject-default-varchar-too-long" => SqlDefError::DefaultOutOfRange,
        "reject-default-text-invalid-utf8" => SqlDefError::BadUtf8,
        "reject-pk-position-out-of-range" => SqlDefError::BadKeyPosition,
        "reject-pk-position-repeated" => SqlDefError::BadKeyPosition,
        "reject-pk-count-above-columns" => SqlDefError::BadKeyPosition,
        "reject-pk-nullable" => SqlDefError::KeyNullable,
        "reject-pk-numeric" => SqlDefError::KeyType,
        "reject-unique-jsonb" => SqlDefError::KeyType,
        "reject-unique-empty" => SqlDefError::BadKeyPosition,
        "reject-pk-empty-name" => SqlDefError::BadConstraintName,
        "reject-constraint-name-256-bytes" => SqlDefError::BadConstraintName,
        "reject-duplicate-constraint-name" => SqlDefError::DuplicateConstraintName,
        "reject-sixteen-uniques" => SqlDefError::TooManyUniques,
        "reject-trailing-byte" => SqlDefError::Trailing,
        "reject-unique-count-short" => SqlDefError::Truncated,
        "reject-name-length-past-end" => SqlDefError::Truncated,
        "reject-default-length-past-end" => SqlDefError::Truncated,
        "reject-column-count-past-end" => SqlDefError::Truncated,
    );
    assert_eq!(cases.len(), 41);
    for (name, _, expected, got) in cases {
        assert_eq!(got, Some(expected), "{name}");
    }
}

#[test]
fn every_block_rejection_fixture_is_refused_for_its_rule() {
    let cases = rejects!(sqldef::decode_ops;
        "ops-reject-empty" => SqlDefError::EmptyDefinitions,
        "ops-reject-count-1025" => SqlDefError::TooManyDefinitions,
        "ops-reject-op-tag-2" => SqlDefError::BadOpTag(2),
        "ops-reject-table-id-over-limit" => SqlDefError::BadTableId(1 << 26),
        "ops-reject-not-ascending" => SqlDefError::NotAscending,
        "ops-reject-duplicate-id" => SqlDefError::DuplicateTableId(5),
        "ops-reject-body-over-64kib" => SqlDefError::TooLarge,
        "ops-reject-body-length-past-end" => SqlDefError::Truncated,
        "ops-reject-trailing-byte" => SqlDefError::Trailing,
        "ops-reject-count-past-end" => SqlDefError::Truncated,
    );
    for (name, _, expected, got) in cases {
        assert_eq!(got, Some(expected), "{name}");
    }
}

const ACCEPTED_BODIES: [&[u8]; 6] = [
    golden!("ok-minimal"),
    golden!("ok-all-types"),
    golden!("ok-all-defaults"),
    golden!("ok-composite-key"),
    golden!("ok-fifteen-uniques"),
    golden!("ok-non-ascii-name"),
];
const ACCEPTED_BLOCKS: [&[u8]; 2] = [golden!("ops-ok-two"), golden!("ops-ok-max-table-id")];

#[test]
fn every_strict_prefix_of_every_accepted_fixture_is_refused() {
    for bytes in ACCEPTED_BODIES {
        for cut in 0..bytes.len() {
            assert!(SqlTableDef::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
    }
    for bytes in ACCEPTED_BLOCKS {
        for cut in 0..bytes.len() {
            assert!(sqldef::decode_ops(&bytes[..cut]).is_err(), "cut {cut}");
        }
    }
}

#[test]
fn a_flipped_bit_is_refused_or_decodes_to_the_changed_bytes() {
    for bytes in ACCEPTED_BODIES {
        for i in 0..bytes.len() {
            for bit in 0..8 {
                let mut mutated = bytes.to_vec();
                mutated[i] ^= 1 << bit;
                if let Ok(def) = SqlTableDef::decode(&mutated) {
                    assert_eq!(def.encode().unwrap(), mutated, "byte {i} bit {bit}");
                }
            }
        }
    }
    for bytes in ACCEPTED_BLOCKS {
        for i in 0..bytes.len() {
            for bit in 0..8 {
                let mut mutated = bytes.to_vec();
                mutated[i] ^= 1 << bit;
                if let Ok(ops) = sqldef::decode_ops(&mutated) {
                    assert_eq!(
                        sqldef::encode_ops(&ops).unwrap(),
                        mutated,
                        "byte {i} bit {bit}"
                    );
                }
            }
        }
    }
}

#[test]
fn lowercase_and_uppercase_non_ascii_names_are_both_accepted() {
    let def = SqlTableDef::decode(golden!("ok-non-ascii-name")).unwrap();
    let names: Vec<&str> = def.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["\u{e9}", "\u{c9}"]);
    assert_eq!(def.encode().unwrap(), golden!("ok-non-ascii-name"));
}

#[test]
fn a_body_over_the_cap_is_refused_before_it_is_parsed() {
    let mut big = golden!("ok-minimal").to_vec();
    big.resize(sqldef::MAX_BODY + 1, 0);
    assert_eq!(SqlTableDef::decode(&big), Err(SqlDefError::TooLarge));
}

#[test]
fn the_encoder_refuses_what_the_decoder_would() {
    let mut upper = minimal();
    upper.columns[0].name = "Id".into();
    assert_eq!(upper.encode(), Err(SqlDefError::BadName));
    let mut bad_nan = minimal();
    bad_nan.columns[0] = Column {
        ty: SqlType::Double,
        default: Some(DefaultValue::Double(0xfff8_0000_0000_0000)),
        ..column("x", SqlType::Double)
    };
    assert_eq!(bad_nan.encode(), Err(SqlDefError::DefaultOutOfRange));
    let mut zero_key = composite();
    zero_key.uniques[0].columns.clear();
    assert_eq!(zero_key.encode(), Err(SqlDefError::BadKeyPosition));
    let mut big = minimal();
    // 255-byte names reach the body cap long before the 1024 column limit.
    big.columns = (0..300)
        .map(|i| column(&format!("{i:0>250}"), SqlType::Text))
        .collect();
    assert_eq!(big.encode(), Err(SqlDefError::TooLarge));
    assert_eq!(sqldef::encode_ops(&[]), Err(SqlDefError::EmptyDefinitions));
    let over = DefineTable {
        table_id: sqldef::MAX_TABLE_ID + 1,
        definition: minimal(),
    };
    assert_eq!(
        sqldef::encode_ops(&[over]),
        Err(SqlDefError::BadTableId(1 << 26))
    );
}

/// Planted bug `SqlDefColumnsSorted`: a decoder that sorts columns by name loses the declaration
/// order that is the row layout. The witness decodes a definition whose columns are not sorted.
#[test]
fn the_column_sort_planted_bug_is_caught_by_the_layout_witness() {
    let _guard = BugGuard;
    let unsorted = SqlTableDef {
        columns: vec![
            column("zeta", SqlType::Integer),
            column("alpha", SqlType::Text),
        ],
        primary_key: None,
        uniques: Vec::new(),
    };
    let bytes = unsorted.encode().unwrap();
    for mutated in [false, true] {
        bugs::clear();
        if mutated {
            bugs::enable(Bug::SqlDefColumnsSorted);
        }
        let decoded = SqlTableDef::decode(&bytes).unwrap();
        let names: Vec<&str> = decoded.columns.iter().map(|c| c.name.as_str()).collect();
        if mutated {
            assert_eq!(names, ["alpha", "zeta"]);
            assert_ne!(decoded.encode().unwrap(), bytes);
        } else {
            assert_eq!(names, ["zeta", "alpha"]);
            assert_eq!(decoded.encode().unwrap(), bytes);
        }
    }
}

fn text_default(len: usize) -> SqlTableDef {
    let mut c = column("a", SqlType::Text);
    c.default = Some(DefaultValue::Text("x".repeat(len)));
    SqlTableDef {
        columns: vec![c],
        primary_key: None,
        uniques: Vec::new(),
    }
}

#[test]
fn the_column_count_boundary_is_1024() {
    let many = |n: usize| SqlTableDef {
        columns: (0..n)
            .map(|i| column(&format!("c{i}"), SqlType::Integer))
            .collect(),
        primary_key: None,
        uniques: Vec::new(),
    };
    let ok = many(1024).encode().unwrap();
    assert_eq!(SqlTableDef::decode(&ok).unwrap().columns.len(), 1024);
    assert_eq!(many(1025).encode(), Err(SqlDefError::BadColumnCount));
    // Raw header declaring 1025 columns: refused on the count, before any column is read.
    assert_eq!(
        SqlTableDef::decode(&[1, 1, 0x01, 0x04]),
        Err(SqlDefError::BadColumnCount)
    );
}

#[test]
fn the_name_length_boundary_is_255_bytes() {
    let mut def = minimal();
    def.columns[0].name = "a".repeat(255);
    assert_eq!(SqlTableDef::decode(&def.encode().unwrap()).unwrap(), def);
    def.columns[0].name = "a".repeat(256);
    assert_eq!(def.encode(), Err(SqlDefError::BadName));
    let mut keyed = composite();
    keyed.uniques[0].name = "u".repeat(255);
    assert_eq!(
        SqlTableDef::decode(&keyed.encode().unwrap()).unwrap(),
        keyed
    );
    keyed.uniques[0].name = "u".repeat(256);
    assert_eq!(keyed.encode(), Err(SqlDefError::BadConstraintName));
}

#[test]
fn the_body_size_boundary_is_65536_bytes() {
    let base = text_default(0).encode().unwrap().len();
    let exact = text_default(sqldef::MAX_BODY - base);
    let bytes = exact.encode().unwrap();
    assert_eq!(bytes.len(), sqldef::MAX_BODY);
    assert_eq!(SqlTableDef::decode(&bytes).unwrap(), exact);
    assert_eq!(
        text_default(sqldef::MAX_BODY - base + 1).encode(),
        Err(SqlDefError::TooLarge)
    );
    let mut over = bytes;
    over.push(0);
    assert_eq!(SqlTableDef::decode(&over), Err(SqlDefError::TooLarge));
}

#[test]
fn the_block_size_boundary_is_1024_operations() {
    let ops = |n: u32| -> Vec<DefineTable> {
        (0..n)
            .map(|table_id| DefineTable {
                table_id,
                definition: minimal(),
            })
            .collect()
    };
    let bytes = sqldef::encode_ops(&ops(1024)).unwrap();
    assert_eq!(sqldef::decode_ops(&bytes).unwrap(), ops(1024));
    assert_eq!(
        sqldef::encode_ops(&ops(1025)),
        Err(SqlDefError::TooManyDefinitions)
    );
}

#[test]
fn unique_constraint_names_must_be_nonempty_and_distinct() {
    let mut empty = composite();
    empty.uniques[0].name.clear();
    assert_eq!(empty.encode(), Err(SqlDefError::BadConstraintName));
    let mut twice = composite();
    twice.uniques = vec![key(&[2], "same"), key(&[1], "same")];
    assert_eq!(twice.encode(), Err(SqlDefError::DuplicateConstraintName));
}

// RFC 0012 amendment proposals (issue 642). The codec follows the amendment once it lands on the
// RFC branch; until then these state the proposed behaviour and are ignored.
#[test]
#[ignore = "RFC 0012 amendment #642 pending"]
fn reject_varchar_limit_over_10485760() {
    let mut def = minimal();
    def.columns[0].ty = SqlType::Varchar;
    def.columns[0].varchar_limit = 10_485_760;
    assert!(def.encode().is_ok());
    def.columns[0].varchar_limit = 10_485_761;
    assert!(def.encode().is_err());
}

#[test]
#[ignore = "RFC 0012 amendment #642 pending"]
fn reject_unique_equals_primary_key() {
    let mut def = composite();
    def.uniques = vec![key(&[1, 0], "same_set")];
    assert!(def.encode().is_err());
}

#[test]
#[ignore = "RFC 0012 amendment #642 pending"]
fn reject_duplicate_unique_sets() {
    let mut def = composite();
    def.uniques = vec![key(&[2], "a"), key(&[2], "b")];
    assert!(def.encode().is_err());
}
