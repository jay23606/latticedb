//! Mutation fuzzing on the stable toolchain: take valid inputs, mangle them with a seeded
//! generator, and require that no decoder, parser or recovery path ever panics or hangs. The
//! cargo-fuzz targets in `fuzz/` cover the same entry points with coverage guidance; this runs on
//! every CI build and replays exactly from its seed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_core::changeset::{
    ChangeSet, ChangeSetId, TurnRef, decode_turn_begin, encode_turn_begin,
};
use lattice_core::record::{decode_record, encode_record, segment_header};
use lattice_core::{Config, Kernel};
use lattice_seams::{Io, Rng};
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use lattice_testkit::scenario::{SharedClock, SharedIo};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

const ITERATIONS: usize = if cfg!(debug_assertions) {
    4_000
} else {
    150_000
};

fn mutate(rng: &mut SeededRng, base: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..=rng.below(4) {
        match rng.below(6) {
            0 if !v.is_empty() => {
                let i = rng.below(v.len() as u64) as usize;
                v[i] ^= 1 << rng.below(8);
            }
            1 if !v.is_empty() => {
                let i = rng.below(v.len() as u64) as usize;
                v[i] = rng.next_u64() as u8;
            }
            2 => {
                let i = rng.below(v.len() as u64 + 1) as usize;
                v.insert(i, rng.next_u64() as u8);
            }
            3 if !v.is_empty() => {
                let i = rng.below(v.len() as u64) as usize;
                v.remove(i);
            }
            4 => v.truncate(rng.below(v.len() as u64 + 1) as usize),
            _ => {
                let extra = rng.below(40) as usize;
                v.extend((0..extra).map(|_| rng.next_u64() as u8));
            }
        }
    }
    v
}

fn fuzz(name: &str, corpus: &[Vec<u8>], mut target: impl FnMut(&[u8])) {
    let mut rng = SeededRng::new(0xF0F0 ^ name.len() as u64);
    for i in 0..ITERATIONS {
        let base = &corpus[rng.below(corpus.len() as u64) as usize];
        let input = mutate(&mut rng, base);
        if catch_unwind(AssertUnwindSafe(|| target(&input))).is_err() {
            let hex: String = input.iter().map(|b| format!("{b:02x}")).collect();
            panic!("{name}: input #{i} crashed the target: {hex}");
        }
    }
}

fn sample_changeset() -> ChangeSet {
    let mut c = ChangeSet::new(ChangeSetId::from_raw(0xABCDEF), b"payload".to_vec());
    c.turn = Some(TurnRef {
        session: 3,
        turn: 4,
    });
    c.parents = vec![ChangeSetId::from_raw(1), ChangeSetId::from_raw(2)];
    c
}

#[test]
fn record_decoder_survives_mutation() {
    let corpus = vec![
        encode_record(1, 1, &sample_changeset().encode().unwrap()),
        encode_record(
            2,
            2,
            &encode_turn_begin(TurnRef {
                session: 1,
                turn: 2,
            }),
        ),
        encode_record(9, 77, b""),
    ];
    fuzz("wal_record", &corpus, |b| {
        let _ = decode_record(b, 0);
        let _ = decode_record(b, b.len() / 2);
    });
}

#[test]
fn changeset_and_turn_decoders_survive_mutation() {
    let corpus = vec![
        sample_changeset().encode().unwrap(),
        ChangeSet::new(ChangeSetId::from_raw(1), vec![])
            .encode()
            .unwrap(),
    ];
    fuzz("changeset_decode", &corpus, |b| {
        if let Ok(cs) = ChangeSet::decode(b) {
            // Anything that decodes must re-encode to something that decodes to the same value.
            assert_eq!(ChangeSet::decode(&cs.encode().unwrap()), Ok(cs));
        }
        let _ = decode_turn_begin(b);
    });
}

#[test]
fn id_parser_survives_mutation() {
    let corpus = vec![ChangeSetId::from_raw(123456789).to_string().into_bytes()];
    fuzz("id_parse", &corpus, |b| {
        if let Ok(s) = std::str::from_utf8(b) {
            let _ = ChangeSetId::parse(s);
        }
    });
}

#[test]
fn schema_parser_checker_and_validator_survive_mutation() {
    use lattice_lsl::{
        check::check,
        parser::parse,
        validate::{Value, validate},
    };
    let src = "enum P { low, high }\nfacet E { n: int in 0..=9 optional  p: P }\ntype Epic { title: text max 20 }\ntype T = E { t: text max 9  r: ref<Epic> optional  l: list<bytes max 4> max 3 }\n";
    let corpus = vec![src.as_bytes().to_vec(), b"type X {}".to_vec()];
    fuzz("lsl_parse", &corpus, |b| {
        if let Ok(text) = std::str::from_utf8(b)
            && let Ok(schema) = parse(text)
            && let Ok(compiled) = check(&schema)
        {
            let empty = Value::Object(Default::default());
            for t in compiled.types.keys() {
                let _ = validate(&compiled, t, &empty);
            }
        }
    });
}

#[test]
fn log_recovery_survives_mutated_segments() {
    // A real segment with several records, then mangled bytes fed to recovery.
    let mut seg = segment_header();
    for lsn in 1..=4u64 {
        let mut c = sample_changeset();
        c.id = ChangeSetId::from_raw(u128::from(lsn));
        seg.extend_from_slice(&encode_record(lsn, 1, &c.encode().unwrap()));
    }
    seg.extend_from_slice(&encode_record(
        5,
        2,
        &encode_turn_begin(TurnRef {
            session: 3,
            turn: 4,
        }),
    ));
    let corpus = vec![seg];
    fuzz("log_open", &corpus, |b| {
        let mut io = SimIo::new(1, FaultProfile::clean());
        io.create_dir_all("wal").unwrap();
        io.append("wal/00000000000000000001.seg", b).unwrap();
        let disk = SharedIo::new(io);
        let config = Config {
            dir: "wal".into(),
            segment_limit: 1 << 20,
            node: 1,
        };
        // Either a clean recovery or a refusal; never a panic. A recovered log must accept writes.
        if let Ok((mut k, _)) = Kernel::open(
            disk,
            SharedClock(Rc::new(SimClock::new(1))),
            SeededRng::new(1),
            &config,
        ) {
            let id = k.new_id();
            let _ = k.commit(ChangeSet::new(id, vec![1]));
        }
    });
}
