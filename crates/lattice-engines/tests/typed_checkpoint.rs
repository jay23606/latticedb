//! Combined schemas/rows must restore from the checkpoint itself, including original layouts.
#![cfg(feature = "row")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use lattice_core::Config;
use lattice_engines::mvcc::Snapshot;
use lattice_engines::store::RowStore;
use lattice_engines::typed_store::TypedStore;
use lattice_lsl::validate::Value;
use lattice_seams::Io;
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use lattice_testkit::scenario::{SharedClock, SharedIo};
use std::rc::Rc;

type Store = TypedStore<SharedIo, SharedClock, SeededRng>;
const V1: &str = "type Ticket { points: int in 0..=5 }";
const V2: &str = "type Ticket { aaa: bool, points: int in 0..=10 }";
const V3: &str = "type New { title: text } type Ticket { aaa: bool, points: int in 0..=10 }";
fn config() -> Config {
    Config {
        dir: "wal".into(),
        segment_limit: 512,
        node: 1,
    }
}
fn open(disk: &SharedIo, clock: &Rc<SimClock>, seed: u64) -> Store {
    TypedStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(seed),
        &config(),
    )
    .unwrap()
}
fn object(points: i64, added: bool) -> Value {
    let mut fields = std::collections::BTreeMap::from([("points".into(), Value::Int(points))]);
    if added {
        fields.insert("aaa".into(), Value::Bool(true));
    }
    Value::Object(fields)
}
fn prepare() -> (SharedIo, Rc<SimClock>, Snapshot, u64) {
    let disk = SharedIo::new(SimIo::new(7, FaultProfile::clean()));
    let clock = Rc::new(SimClock::new(1000));
    let mut store = open(&disk, &clock, 7);
    store.install_schema(V1).unwrap();
    let mut tx = store.begin();
    store
        .put(&mut tx, "Ticket", b"old", &object(3, false))
        .unwrap();
    store.commit(tx).unwrap();
    let historical = store.snapshot();
    store.install_schema(V2).unwrap();
    let mut tx = store.begin();
    store
        .put(&mut tx, "Ticket", b"new", &object(8, true))
        .unwrap();
    store.commit(tx).unwrap();
    drop(store);
    let mut raw = RowStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(8),
        &config(),
    )
    .unwrap();
    let mut deleted = raw.begin();
    deleted.delete((2, b"raw tombstone".to_vec()));
    raw.commit(deleted).unwrap();
    let through = raw.snapshot().as_of;
    drop(raw);
    (disk, clock, historical, through)
}
fn assert_values(store: &Store, historical: Snapshot) {
    assert_eq!(
        store.get_at(historical, "Ticket", b"old").unwrap(),
        Some(object(3, false))
    );
    assert_eq!(
        store.get_at(store.snapshot(), "Ticket", b"old").unwrap(),
        Some(object(3, false))
    );
    assert_eq!(
        store.get_at(store.snapshot(), "Ticket", b"new").unwrap(),
        Some(object(8, true))
    );
    assert!(store.catalog().schema(1).is_some());
    assert!(store.catalog().schema(2).is_some());
}

#[test]
fn fast_checkpoint_restores_original_layouts_and_raw_tombstone_reservations() {
    let (disk, clock, historical, through) = prepare();
    let mut store = open(&disk, &clock, 9);
    let seal = store.checkpoint().unwrap();
    assert_eq!(seal, through + 1);
    drop(store);
    disk.crash();
    let mut store = open(&disk, &clock, 10);
    // A full-WAL fallback must not let this test pass as evidence of snapshot schema restoration.
    assert_eq!(store.recovery_report().1.through_lsn, through);
    assert!(store.recovery_report().1.skipped_wal_bytes > 0);
    assert_values(&store, historical);
    store.install_schema(V3).unwrap();
    let table = store
        .catalog()
        .id_of(store.snapshot().as_of, "New")
        .unwrap();
    assert_eq!(
        table, 3,
        "raw table 2 tombstone must still reserve its id after fast restore"
    );
    store.checkpoint().unwrap();
    let second = store.snapshot();
    drop(store);
    disk.crash();
    let store = open(&disk, &clock, 11);
    assert_eq!(store.snapshot(), second);
    assert_values(&store, historical);
    assert!(store.catalog().schema(3).is_some());
}

#[test]
fn corrupted_combined_chunks_fall_back_to_full_catalog_and_row_replay() {
    let (disk, clock, historical, _) = prepare();
    let mut store = open(&disk, &clock, 12);
    store.checkpoint().unwrap();
    drop(store);
    let chunk = disk
        .list("wal/.checkpoint")
        .unwrap()
        .into_iter()
        .find(|name| name.ends_with(".chunk"))
        .unwrap();
    let path = format!("wal/.checkpoint/{chunk}");
    let mut data = disk.read(&path).unwrap();
    *data.first_mut().unwrap() ^= 1;
    let mut writer = disk.clone();
    writer.truncate(&path, 0).unwrap();
    writer.append(&path, &data).unwrap();
    writer.sync(&path).unwrap();
    disk.crash();
    let store = open(&disk, &clock, 13);
    assert_eq!(store.recovery_report().1.through_lsn, 0);
    assert_values(&store, historical);
}

#[test]
fn every_combined_checkpoint_publication_cut_preserves_acknowledged_schema_rows() {
    let (baseline, clock, _, _) = prepare();
    let mut store = open(&baseline, &clock, 14);
    let before = baseline.mutating_ops();
    store.checkpoint().unwrap();
    let operations = baseline.mutating_ops() - before;
    drop(store);
    assert!(operations > 3);
    for cut in 0..=operations {
        let (disk, clock, historical, _) = prepare();
        let mut store = open(&disk, &clock, 14);
        disk.arm_power_cut(cut);
        let acknowledged = store.checkpoint().is_ok();
        drop(store);
        disk.crash();
        let recovered = open(&disk, &clock, 15);
        assert_values(&recovered, historical);
        if acknowledged {
            assert!(
                recovered.recovery_report().1.through_lsn > 0,
                "acknowledged seal must survive, cut={cut}"
            );
        }
    }
}

#[test]
fn catalog_only_checkpoint_preserves_every_immutable_schema_after_drop() {
    use lattice_engines::catalog::{CatalogOp, CatalogStore};
    use lattice_lsl::{format, parser};
    let disk = SharedIo::new(SimIo::new(23, FaultProfile::clean()));
    let clock = Rc::new(SimClock::new(1000));
    let mut store = CatalogStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(23),
        &config(),
    )
    .unwrap();
    let mut lsns = Vec::new();
    for (version, source) in [(1, V1), (2, V2)] {
        let source = format::format(&parser::parse(source).unwrap());
        lsns.push(
            store
                .commit(vec![CatalogOp::DefineSchema {
                    version,
                    source,
                    tables: std::collections::BTreeMap::from([(1, "Ticket".into())]),
                }])
                .unwrap(),
        );
    }
    let through = store.commit(vec![CatalogOp::Drop { table: 1 }]).unwrap();
    store.checkpoint().unwrap();
    drop(store);
    disk.crash();
    let mut store =
        CatalogStore::open(disk, SharedClock(clock), SeededRng::new(24), &config()).unwrap();
    assert_eq!(store.recovery_report().1.through_lsn, through);
    for (version, lsn) in (1..=2).zip(lsns) {
        assert_eq!(store.catalog().schema(version).unwrap().installed_lsn, lsn);
        assert_eq!(
            store.catalog().layout_at(1, lsn).unwrap().0.version,
            version
        );
    }
    assert!(store.catalog().current().is_empty());
    assert!(
        store
            .commit(vec![CatalogOp::Create {
                table: 1,
                name: "reused".into()
            }])
            .is_err()
    );
}
