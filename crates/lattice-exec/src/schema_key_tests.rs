//! The key encoding: exact bytes for the edge values, and the order and self-delimiting properties
//! that make a scan of the encoded keys a scan of the values. Expected bytes are worked out by hand
//! from the encoding as designed, not by running the code under test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use super::*;

fn key(d: &Datum) -> Vec<u8> {
    let mut out = Vec::new();
    push_key(&mut out, d).unwrap();
    out
}

fn bytes(hex: &[u8]) -> Vec<u8> {
    hex.to_vec()
}

#[test]
fn integers_are_big_endian_with_the_sign_bit_flipped() {
    assert_eq!(key(&Datum::Int(0)), bytes(&[0x80, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(key(&Datum::Int(1)), bytes(&[0x80, 0, 0, 0, 0, 0, 0, 1]));
    assert_eq!(
        key(&Datum::Int(-1)),
        bytes(&[0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
    );
    assert_eq!(key(&Datum::Int(i64::MIN)), bytes(&[0; 8]));
    assert_eq!(key(&Datum::Int(i64::MAX)), bytes(&[0xFF; 8]));
    assert_eq!(
        key(&Datum::Int(0x0102_0304_0506_0708)),
        bytes(&[0x81, 2, 3, 4, 5, 6, 7, 8])
    );
    assert_eq!(key(&Datum::Timestamp(-1)), key(&Datum::Int(-1)));
}

#[test]
fn dates_are_four_bytes_with_the_sign_bit_flipped() {
    assert_eq!(key(&Datum::Date(0)), bytes(&[0x80, 0, 0, 0]));
    assert_eq!(key(&Datum::Date(-1)), bytes(&[0x7F, 0xFF, 0xFF, 0xFF]));
    assert_eq!(key(&Datum::Date(i32::MIN)), bytes(&[0; 4]));
    assert_eq!(key(&Datum::Date(i32::MAX)), bytes(&[0xFF; 4]));
    assert_eq!(key(&Datum::Date(0x0102_0304)), bytes(&[0x81, 2, 3, 4]));
}

#[test]
fn floats_order_as_numbers_and_zero_has_one_encoding() {
    // 1.0 is 0x3FF0_0000_0000_0000: the sign bit is flipped. -1.0 is 0xBFF0_...: all bits are
    // inverted.
    assert_eq!(
        key(&Datum::Float(1.0)),
        bytes(&[0xBF, 0xF0, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        key(&Datum::Float(-1.0)),
        bytes(&[0x40, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
    );
    assert_eq!(key(&Datum::Float(0.0)), bytes(&[0x80, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(key(&Datum::Float(-0.0)), key(&Datum::Float(0.0)));
    // NaN sorts above infinity, and every NaN has the same key.
    assert_eq!(
        key(&Datum::Float(f64::INFINITY)),
        bytes(&[0xFF, 0xF0, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        key(&Datum::Float(f64::NAN)),
        bytes(&[0xFF, 0xF8, 0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        key(&Datum::Float(f64::from_bits(0xFFF8_0000_0000_0001))),
        key(&Datum::Float(f64::NAN))
    );
    let ascending = [
        f64::NEG_INFINITY,
        -1.0e300,
        -1.5,
        -1.0,
        -f64::MIN_POSITIVE,
        0.0,
        f64::MIN_POSITIVE,
        1.0,
        1.5,
        1.0e300,
        f64::INFINITY,
        f64::NAN,
    ];
    for pair in ascending.windows(2) {
        assert!(
            key(&Datum::Float(pair[0])) < key(&Datum::Float(pair[1])),
            "{} then {}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn booleans_and_uuids_have_fixed_widths() {
    assert_eq!(key(&Datum::Bool(false)), bytes(&[0]));
    assert_eq!(key(&Datum::Bool(true)), bytes(&[1]));
    let u = 0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10_u128;
    assert_eq!(
        key(&Datum::Uuid(u)),
        bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
    );
}

#[test]
fn text_and_bytes_escape_zero_and_end_with_two_zeros() {
    assert_eq!(key(&Datum::Text(String::new())), bytes(&[0, 0]));
    assert_eq!(key(&Datum::Text("a".into())), bytes(&[0x61, 0, 0]));
    assert_eq!(
        key(&Datum::Text("a\0b".into())),
        bytes(&[0x61, 0, 0xFF, 0x62, 0, 0])
    );
    assert_eq!(key(&Datum::Bytes(vec![0])), bytes(&[0, 0xFF, 0, 0]));
    assert_eq!(
        key(&Datum::Bytes(vec![0, 0])),
        bytes(&[0, 0xFF, 0, 0xFF, 0, 0])
    );
    assert_eq!(key(&Datum::Bytes(vec![1, 0xFF])), bytes(&[1, 0xFF, 0, 0]));
}

#[test]
fn a_shorter_string_sorts_before_its_extensions_and_nothing_collides() {
    let values: Vec<Vec<u8>> = vec![
        vec![],
        vec![0],
        vec![0, 0],
        vec![0, 1],
        vec![1],
        vec![1, 0],
        vec![1, 0, 0],
        vec![1, 1],
        vec![0xFF],
        vec![0xFF, 0],
    ];
    let mut sorted = values.clone();
    sorted.sort();
    let keys: Vec<Vec<u8>> = sorted
        .iter()
        .map(|v| key(&Datum::Bytes(v.clone())))
        .collect();
    for pair in keys.windows(2) {
        assert!(pair[0] < pair[1], "{pair:?}");
    }
}

#[test]
fn a_composite_key_is_the_concatenation_and_cannot_be_confused() {
    // ("a", "bc") and ("ab", "c") must differ, which only the terminator guarantees.
    let one = key_of(
        &[Datum::Text("a".into()), Datum::Text("bc".into())],
        &[0, 1],
    )
    .unwrap();
    let two = key_of(
        &[Datum::Text("ab".into()), Datum::Text("c".into())],
        &[0, 1],
    )
    .unwrap();
    assert_ne!(one, two);
    assert_eq!(one, bytes(&[0x61, 0, 0, 0x62, 0x63, 0, 0]));
    // The columns are taken in the order of the index list, not the row.
    let swapped = key_of(
        &[Datum::Text("a".into()), Datum::Text("bc".into())],
        &[1, 0],
    )
    .unwrap();
    assert_eq!(swapped, bytes(&[0x62, 0x63, 0, 0, 0x61, 0, 0]));
    assert!(key_of(&[Datum::Int(1)], &[1]).is_err());
}

#[test]
fn null_numeric_and_json_cannot_be_part_of_a_key() {
    for d in [
        Datum::Null,
        Datum::Json("1".into()),
        Datum::Numeric(Decimal::parse("1").unwrap()),
    ] {
        let mut out = vec![9];
        assert!(push_key(&mut out, &d).is_err(), "{d:?}");
    }
}

#[test]
fn the_owning_table_of_a_marker_is_found_from_its_id() {
    // Marker ids are bit 31, the table id shifted left four, and the constraint number.
    assert!(is_marker_table(0x8000_0072));
    assert_eq!(owning_table(0x8000_0072), 7);
    assert_eq!(owning_table(0x8000_0070), 7);
    assert_eq!(owning_table(0x8000_007F), 7);
    assert_eq!(owning_table(0x8000_0010), 1);
    assert_eq!(
        owning_table(0x8000_0000 | (MAX_TABLE_ID << 4) | 15),
        MAX_TABLE_ID
    );
    // An ordinary id is its own owner, up to the largest the bit leaves.
    for id in [0, 1, 7, MAX_TABLE_ID, 0x7FFF_FFFF] {
        assert!(!is_marker_table(id), "{id}");
        assert_eq!(owning_table(id), id);
    }
}

fn def(id: u32, sql: &str) -> TableDef {
    let Ok(lattice_sql::ast::Statement::CreateTable(ct)) = lattice_sql::parse_statement(sql) else {
        panic!("`{sql}` is not a CREATE TABLE");
    };
    TableDef::from_create(id, &ct).unwrap()
}

#[test]
fn the_generation_counts_each_change_to_the_set_of_tables() {
    let mut schema = Schema::new();
    assert_eq!(schema.generation(), 0);
    assert_eq!(schema.tables().count(), 0);

    schema.insert(def(1, "CREATE TABLE b (x INT)"));
    assert_eq!(schema.generation(), 1);
    schema.insert(def(2, "CREATE TABLE a (x INT)"));
    assert_eq!(schema.generation(), 2);
    let names: Vec<&str> = schema.tables().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);

    // Removing a table that is not there changes nothing.
    assert!(schema.remove("nope").is_none());
    assert_eq!(schema.generation(), 2);
    let gone = schema.remove("a").unwrap();
    assert_eq!(gone.id, 2);
    assert_eq!(schema.generation(), 3);
    assert_eq!(schema.remove("a").map(|t| t.id), None);
    assert_eq!(schema.generation(), 3);
    assert_eq!(schema.tables().count(), 1);
    assert_eq!(schema.by_id(1).map(|t| t.name.as_str()), Some("b"));
    assert!(schema.by_id(2).is_none());
}
