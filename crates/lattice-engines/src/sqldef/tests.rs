#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use super::*;
use proptest::prelude::*;

fn default_for(ty: SqlType) -> BoxedStrategy<DefaultValue> {
    match ty {
        SqlType::Boolean => any::<bool>().prop_map(DefaultValue::Bool).boxed(),
        SqlType::SmallInt => any::<i16>()
            .prop_map(|v| DefaultValue::Int(v.into()))
            .boxed(),
        SqlType::Integer => any::<i32>()
            .prop_map(|v| DefaultValue::Int(v.into()))
            .boxed(),
        SqlType::BigInt => any::<i64>().prop_map(DefaultValue::Int).boxed(),
        SqlType::Double => any::<u64>()
            .prop_map(|bits| {
                DefaultValue::Double(if f64::from_bits(bits).is_nan() {
                    CANONICAL_NAN
                } else {
                    bits
                })
            })
            .boxed(),
        SqlType::Numeric => "0|-?[1-9][0-9]{0,5}(\\.[0-9]{0,3}[1-9])?|-?0\\.[0-9]{0,3}[1-9]"
            .prop_map(DefaultValue::Numeric)
            .boxed(),
        SqlType::Text | SqlType::Varchar => "\\PC{0,8}".prop_map(DefaultValue::Text).boxed(),
        SqlType::Bytea => proptest::collection::vec(any::<u8>(), 0..8)
            .prop_map(DefaultValue::Bytes)
            .boxed(),
        SqlType::Uuid => any::<[u8; 16]>().prop_map(DefaultValue::Uuid).boxed(),
        SqlType::Date => any::<i32>().prop_map(DefaultValue::Date).boxed(),
        SqlType::Timestamptz => any::<i64>().prop_map(DefaultValue::Timestamptz).boxed(),
        SqlType::Jsonb => "\\PC{0,12}".prop_map(DefaultValue::Jsonb).boxed(),
    }
}

fn column(index: usize) -> impl Strategy<Value = Column> {
    (
        proptest::char::range('a', 'z'),
        1u8..=13,
        any::<bool>(),
        any::<bool>(),
        8u32..40,
        any::<bool>(),
    )
        .prop_flat_map(
            move |(prefix, tag, not_null, has_default, limit, limited)| {
                let ty = SqlType::from_tag(tag).unwrap();
                let varchar_limit = if ty == SqlType::Varchar && limited {
                    limit
                } else {
                    0
                };
                let default = if has_default {
                    default_for(ty).prop_map(Some).boxed()
                } else {
                    Just(None).boxed()
                };
                default.prop_map(move |default| Column {
                    // A random leading letter makes the names unsorted, so ordering bugs show.
                    name: format!("{prefix}{index}"),
                    ty,
                    varchar_limit,
                    not_null,
                    default,
                })
            },
        )
}

fn definition() -> impl Strategy<Value = SqlTableDef> {
    (1usize..7)
        .prop_flat_map(|n| (0..n).map(column).collect::<Vec<_>>())
        .prop_flat_map(|columns| {
            let keyable: Vec<u16> = (0..columns.len() as u16)
                .filter(|&p| columns[usize::from(p)].ty.can_be_key())
                .collect();
            let primary: Vec<u16> = keyable
                .iter()
                .copied()
                .filter(|&p| columns[usize::from(p)].not_null)
                .collect();
            let pk = proptest::sample::subsequence(primary.clone(), 0..=primary.len());
            // Without a key-capable column there is nothing to constrain: no uniques.
            let uniques = if keyable.is_empty() {
                Just(Vec::new()).boxed()
            } else {
                proptest::collection::vec(
                    proptest::sample::subsequence(keyable.clone(), 1..=keyable.len()),
                    0..4,
                )
                .boxed()
            };
            (Just(columns), pk, uniques)
        })
        .prop_map(|(columns, pk, uniques)| SqlTableDef {
            columns,
            primary_key: (!pk.is_empty()).then(|| KeyConstraint {
                columns: pk,
                name: "pk".into(),
            }),
            uniques: uniques
                .into_iter()
                .enumerate()
                .map(|(k, columns)| KeyConstraint {
                    columns,
                    name: format!("u{k}"),
                })
                .collect(),
        })
}

proptest! {
    #[test]
    fn generated_definitions_round_trip_and_re_encode_identically(def in definition()) {
        let bytes = def.encode().unwrap();
        let decoded = SqlTableDef::decode(&bytes).unwrap();
        prop_assert_eq!(&decoded, &def);
        prop_assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn every_strict_prefix_is_refused(def in definition()) {
        let bytes = def.encode().unwrap();
        for cut in 0..bytes.len() {
            prop_assert!(SqlTableDef::decode(&bytes[..cut]).is_err(), "cut {}", cut);
        }
    }

    #[test]
    fn a_flipped_bit_is_refused_or_decodes_to_the_changed_bytes(
        def in definition(),
        at in any::<prop::sample::Index>(),
        bit in 0u8..8,
    ) {
        let bytes = def.encode().unwrap();
        let mut mutated = bytes.clone();
        let i = at.index(bytes.len());
        mutated[i] ^= 1 << bit;
        if let Ok(other) = SqlTableDef::decode(&mutated) {
            prop_assert_eq!(other.encode().unwrap(), mutated);
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic_and_a_success_re_encodes_identically(
        bytes in proptest::collection::vec(any::<u8>(), 0..300),
    ) {
        if let Ok(def) = SqlTableDef::decode(&bytes) {
            prop_assert_eq!(def.encode().unwrap(), bytes);
        }
    }

    #[test]
    fn generated_blocks_round_trip(
        defs in proptest::collection::vec(definition(), 1..4),
        start in 0u32..1000,
        gaps in proptest::collection::vec(1u32..50, 3),
    ) {
        let mut id = start;
        let ops: Vec<DefineTable> = defs
            .into_iter()
            .zip(gaps)
            .map(|(definition, gap)| {
                id += gap;
                DefineTable { table_id: id, definition }
            })
            .collect();
        let bytes = encode_ops(&ops).unwrap();
        prop_assert_eq!(decode_ops(&bytes).unwrap(), ops);
        for cut in 0..bytes.len() {
            prop_assert!(decode_ops(&bytes[..cut]).is_err());
        }
    }

    #[test]
    fn arbitrary_block_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        if let Ok(ops) = decode_ops(&bytes) {
            prop_assert_eq!(encode_ops(&ops).unwrap(), bytes);
        }
    }
}

#[test]
fn numeric_text_is_canonical_in_exactly_one_spelling() {
    for good in ["0", "-1", "10", "0.5", "-0.5", "123.456", "100"] {
        assert!(canonical_numeric(good), "{good}");
    }
    for bad in [
        "", "-", ".", "-0", "00", "01", "1.", ".5", "1.0", "0.0", "1.50", "+1", "1e3", " 1", "1_0",
        "--1", "NaN",
    ] {
        assert!(!canonical_numeric(bad), "{bad}");
    }
}
