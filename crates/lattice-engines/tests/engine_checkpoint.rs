//! Sealed checkpoint recovery: retained histories, bounded chunks, compatibility and crash sweeps.
#![cfg(feature = "row")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use lattice_core::{AllowAll, ChangeSet, Config, Kernel, Receipt, TurnRef};
use lattice_engines::catalog::{CatalogOp, CatalogStore};
use lattice_engines::index::IndexDef;
use lattice_engines::mvcc::Snapshot;
use lattice_engines::store::RowStore;
use lattice_seams::{Io, IoError};
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use lattice_testkit::scenario::{SharedClock, SharedIo};
use std::{cell::RefCell, rc::Rc};

type Store = RowStore<SharedIo, SharedClock, SeededRng>;
fn config() -> Config {
    Config {
        segment_limit: 512,
        ..Config::default()
    }
}
fn disk() -> SharedIo {
    SharedIo::new(SimIo::new(1, FaultProfile::clean()))
}
fn clock() -> SharedClock {
    SharedClock(Rc::new(SimClock::new(1000)))
}
fn open(disk: &SharedIo) -> Store {
    RowStore::open_with_indexes(
        disk.clone(),
        clock(),
        SeededRng::new(disk.mutating_ops() + 7),
        &config(),
        vec![IndexDef {
            id: 1,
            table: 1,
            extract: |v| Some(v.to_vec()),
        }],
    )
    .unwrap()
}
fn put(store: &mut Store, key: &[u8], value: &[u8]) -> u64 {
    let mut tx = store.begin();
    tx.put((1, key.to_vec()), value.to_vec());
    store.commit(tx).unwrap()
}
fn checkpoint_files(disk: &SharedIo, suffix: &str) -> Vec<String> {
    disk.list("wal/.checkpoint")
        .unwrap()
        .into_iter()
        .filter(|n| n.ends_with(suffix))
        .map(|n| format!("wal/.checkpoint/{n}"))
        .collect()
}
fn replace(disk: &SharedIo, path: &str, bytes: &[u8]) {
    let mut d = disk.clone();
    d.truncate(path, 0).unwrap();
    d.append(path, bytes).unwrap();
    d.sync(path).unwrap();
}

#[test]
fn retained_rows_deletes_indexes_and_new_suffix_survive_repeated_checkpoints() {
    let disk = disk();
    let mut s = open(&disk);
    let first = put(&mut s, b"a", b"one");
    let second = put(&mut s, b"a", b"two");
    let mut tx = s.begin();
    tx.delete((1, b"a".to_vec()));
    let deleted = s.commit(tx).unwrap();
    let seal = s.checkpoint().unwrap();
    assert_eq!(seal, deleted + 1);
    let tail = put(&mut s, b"b", b"three");
    drop(s);
    disk.crash();
    let mut s = open(&disk);
    assert_eq!(s.recovery_report().1.through_lsn, deleted);
    assert_eq!(s.recovery_report().0.records, 2); // seal plus tail
    assert!(s.recovery_report().1.skipped_wal_bytes > 0);
    for (lsn, value) in [
        (first, Some(b"one".to_vec())),
        (second, Some(b"two".to_vec())),
        (deleted, None),
    ] {
        let snap = Snapshot { as_of: lsn };
        assert_eq!(s.get_at(snap, 1, b"a").cloned(), value);
        s.check_indexes_at(snap).unwrap();
    }
    assert_eq!(s.snapshot().as_of, tail);
    s.check_indexes().unwrap();
    s.checkpoint().unwrap();
    drop(s);
    disk.crash();
    let s = open(&disk);
    assert_eq!(s.recovery_report().1.through_lsn, tail);
    assert_eq!(
        s.get_at(Snapshot { as_of: first }, 1, b"a"),
        Some(&b"one".to_vec())
    );
    s.check_indexes_at(Snapshot { as_of: second }).unwrap();
}

#[test]
fn catalog_history_and_dropped_ids_survive_and_wrong_engine_falls_back() {
    let disk = disk();
    let mut c = CatalogStore::open(disk.clone(), clock(), SeededRng::new(1), &config()).unwrap();
    let first = c
        .commit(vec![CatalogOp::Create {
            table: 4,
            name: "before".into(),
        }])
        .unwrap();
    let second = c
        .commit(vec![CatalogOp::Rename {
            table: 4,
            name: "after".into(),
        }])
        .unwrap();
    c.commit(vec![CatalogOp::Drop { table: 4 }]).unwrap();
    c.checkpoint().unwrap();
    drop(c);
    disk.crash();
    let mut c = CatalogStore::open(disk.clone(), clock(), SeededRng::new(2), &config()).unwrap();
    assert!(c.recovery_report().1.through_lsn > 0);
    assert_eq!(c.catalog().id_of(first, "before"), Some(4));
    assert_eq!(c.catalog().id_of(second, "after"), Some(4));
    assert!(
        c.commit(vec![CatalogOp::Create {
            table: 4,
            name: "reuse".into()
        }])
        .is_err()
    );
    drop(c);
    let mut s = open(&disk);
    assert_eq!(s.recovery_report().1.through_lsn, 0);
    put(&mut s, b"raw", b"row");
    s.checkpoint().unwrap();
    drop(s);
    let c = CatalogStore::open(disk.clone(), clock(), SeededRng::new(3), &config()).unwrap();
    assert_eq!(c.recovery_report().1.through_lsn, 0);
    assert_eq!(c.catalog().id_of(first, "before"), Some(4));
}

#[test]
fn corrupted_or_missing_snapshot_falls_back_and_does_not_mask_missing_wal() {
    for damage in 0..3 {
        let disk = disk();
        let mut s = open(&disk);
        let first = put(&mut s, b"a", b"one");
        put(&mut s, b"b", b"two");
        s.checkpoint().unwrap();
        drop(s);
        let file =
            checkpoint_files(&disk, if damage == 1 { ".manifest" } else { ".chunk" }).remove(0);
        if damage == 0 {
            disk.clone().remove(&file).unwrap();
        } else {
            replace(&disk, &file, b"damaged");
        }
        let s = open(&disk);
        assert_eq!(s.recovery_report().1.through_lsn, 0);
        assert_eq!(
            s.get_at(Snapshot { as_of: first }, 1, b"a"),
            Some(&b"one".to_vec())
        );
        drop(s);
        let segment = disk
            .list("wal")
            .unwrap()
            .into_iter()
            .find(|n| n.ends_with(".seg"))
            .unwrap();
        disk.clone().remove(&format!("wal/{segment}")).unwrap();
        assert!(RowStore::open(disk.clone(), clock(), SeededRng::new(2), &config()).is_err());
    }
}

#[test]
fn full_replay_ignores_new_seals_and_scrubs_all_retained_records() {
    let disk = disk();
    let mut s = open(&disk);
    put(&mut s, b"a", b"one");
    s.checkpoint().unwrap();
    put(&mut s, b"b", b"two");
    drop(s);
    disk.crash();
    // This is the unchanged pre-checkpoint kernel opening path: it understands type 3, treats
    // type 4 as opaque, and sees every original changeset because no WAL segment was pruned.
    let (k, recovered) = Kernel::open(disk.clone(), clock(), SeededRng::new(8), &config()).unwrap();
    assert_eq!(recovered.records.len(), 3);
    assert_eq!(recovered.records.iter().filter(|r| r.rtype == 4).count(), 1);
    assert!(k.scrub().unwrap().issues.is_empty());
}

#[test]
fn snapshot_preserves_dedupe_turns_hlc_and_refuses_in_flight() {
    let disk = disk();
    let (mut k, _) = Kernel::open(disk.clone(), clock(), SeededRng::new(1), &config()).unwrap();
    k.begin_turn(TurnRef {
        session: 9,
        turn: 2,
    })
    .unwrap();
    let cs = ChangeSet::new(k.new_id(), b"opaque".to_vec());
    let receipt = k.commit(cs.clone()).unwrap();
    let pending = ChangeSet::new(k.new_id(), vec![]);
    let t = k
        .begin(pending)
        .validate(&AllowAll)
        .unwrap()
        .authorize(&AllowAll)
        .unwrap();
    let lattice_core::PrepareOutcome::Ready(t) = k.prepare(t).unwrap() else {
        panic!()
    };
    let t = k.log(t).unwrap();
    assert!(
        k.checkpoint_engine(b"test-v1", |w| w.bytes(b"state"))
            .is_err()
    );
    let ts = k.sync(vec![t]).unwrap();
    for t in ts {
        k.apply(t).unwrap();
    }
    let cp = k
        .checkpoint_engine(b"test-v1", |w| w.bytes(b"state"))
        .unwrap();
    drop(k);
    disk.crash();
    let (mut k, state, report, evidence) = Kernel::open_checkpointed(
        disk.clone(),
        clock(),
        SeededRng::new(2),
        &config(),
        b"test-v1",
        Vec::new,
        |r, _, _| r.bytes(),
        |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(state, b"state");
    assert_eq!(report.records, 1);
    assert_eq!(evidence.through_lsn, cp - 1);
    assert_eq!(k.commit(cs), Ok(Receipt::Duplicate));
    assert!(
        k.begin_turn(TurnRef {
            session: 9,
            turn: 1
        })
        .is_err()
    );
    let next = ChangeSet::new(k.new_id(), vec![]);
    let new = k.commit(next).unwrap();
    let (Receipt::Recorded { hlc: before, .. }, Receipt::Recorded { hlc: after, .. }) =
        (receipt, new)
    else {
        panic!()
    };
    assert!(after > before);
    assert!(k.scrub().unwrap().issues.is_empty());
}

#[test]
fn publication_survives_every_mutating_power_cut() {
    let baseline = disk();
    let mut s = open(&baseline);
    put(&mut s, b"key", b"acked");
    let before = baseline.mutating_ops();
    s.checkpoint().unwrap();
    let ops = baseline.mutating_ops() - before;
    drop(s);
    assert!(ops > 8);
    for cut in 0..=ops + 1 {
        let disk = disk();
        let mut s = open(&disk);
        let lsn = put(&mut s, b"key", b"acked");
        disk.arm_power_cut(cut);
        let result = s.checkpoint();
        drop(s);
        disk.crash();
        let mut s = open(&disk);
        assert_eq!(
            s.get_at(Snapshot { as_of: lsn }, 1, b"key"),
            Some(&b"acked".to_vec()),
            "cut {cut}: {result:?}"
        );
        s.check_indexes().unwrap();
        put(&mut s, b"next", b"works");
    }
}

#[test]
fn power_loss_during_suffix_sync_never_returns_an_open_store() {
    for cut in 0..3 {
        let disk = disk();
        let mut s = open(&disk);
        put(&mut s, b"key", b"acked");
        s.checkpoint().unwrap();
        drop(s);
        disk.arm_power_cut(cut);
        let result = RowStore::open(disk.clone(), clock(), SeededRng::new(2), &config());
        assert!(result.is_err());
        drop(result);
        disk.crash();
        let s = open(&disk);
        assert_eq!(s.get_at(s.snapshot(), 1, b"key"), Some(&b"acked".to_vec()));
    }
}

#[derive(Default)]
struct Reads {
    bytes: u64,
    largest: usize,
    prefix: usize,
    lists: usize,
}
#[derive(Clone)]
struct CountIo {
    disk: SharedIo,
    reads: Rc<RefCell<Reads>>,
    anchor: u64,
}
impl Io for CountIo {
    fn exists(&self, p: &str) -> bool {
        self.disk.exists(p)
    }
    fn len(&self, p: &str) -> Result<u64, IoError> {
        self.disk.len(p)
    }
    fn read(&self, p: &str) -> Result<Vec<u8>, IoError> {
        let b = self.disk.read(p)?;
        let mut r = self.reads.borrow_mut();
        r.bytes += b.len() as u64;
        r.largest = r.largest.max(b.len());
        if p.ends_with(".seg")
            && p.rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".seg")
                .parse::<u64>()
                .unwrap()
                < self.anchor
        {
            r.prefix += 1;
        }
        Ok(b)
    }
    fn append(&mut self, p: &str, b: &[u8]) -> Result<(), IoError> {
        self.disk.append(p, b)
    }
    fn truncate(&mut self, p: &str, n: u64) -> Result<(), IoError> {
        self.disk.truncate(p, n)
    }
    fn sync(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync(p)
    }
    fn sync_dir(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.sync_dir(p)
    }
    fn list(&self, p: &str) -> Result<Vec<String>, IoError> {
        self.reads.borrow_mut().lists += 1;
        self.disk.list(p)
    }
    fn visit_dir(&self, p: &str, v: &mut dyn FnMut(&str)) -> Result<(), IoError> {
        self.disk.visit_dir(p, v)
    }
    fn remove(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.remove(p)
    }
    fn create_dir_all(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.create_dir_all(p)
    }
    fn lock(&mut self, p: &str) -> Result<(), IoError> {
        self.disk.lock(p)
    }
    fn unlock(&mut self, p: &str) {
        self.disk.unlock(p)
    }
}
#[test]
fn recovery_skips_prefix_io_and_caps_snapshot_read_buffers_as_history_grows() {
    for n in [4, 32] {
        let disk = disk();
        let mut s = RowStore::open(disk.clone(), clock(), SeededRng::new(1), &config()).unwrap();
        for i in 0..n {
            let mut tx = s.begin();
            tx.put((1, vec![i]), vec![i; 100_000]);
            s.commit(tx).unwrap();
        }
        let anchor = s.checkpoint().unwrap();
        drop(s);
        disk.crash();
        let reads = Rc::new(RefCell::new(Reads::default()));
        let s = RowStore::open(
            CountIo {
                disk: disk.clone(),
                reads: reads.clone(),
                anchor,
            },
            clock(),
            SeededRng::new(2),
            &config(),
        )
        .unwrap();
        assert_eq!(s.recovery_report().1.through_lsn, anchor - 1);
        assert_eq!(s.recovery_report().0.records, 1);
        let r = reads.borrow();
        assert_eq!(r.prefix, 0);
        assert_eq!(r.lists, 0);
        assert!(r.largest <= lattice_core::engine_checkpoint::CHUNK_BYTES);
        assert!(s.recovery_report().1.state_bytes > u64::from(n) * 100_000);
    }
}

#[test]
fn losing_every_wal_segment_cannot_turn_a_checkpointed_database_into_an_empty_one() {
    let disk = disk();
    let mut s = open(&disk);
    put(&mut s, b"acked", b"must survive");
    s.checkpoint().unwrap();
    drop(s);
    for name in disk
        .list("wal")
        .unwrap()
        .into_iter()
        .filter(|n| n.ends_with(".seg"))
    {
        disk.clone().remove(&format!("wal/{name}")).unwrap();
    }
    assert!(RowStore::open(disk.clone(), clock(), SeededRng::new(4), &config()).is_err());
    assert!(Kernel::open(disk.clone(), clock(), SeededRng::new(5), &config()).is_err());
}

#[test]
fn missing_all_segments_still_fails_with_only_damaged_manifest_or_orphan_chunk_evidence() {
    for keep_manifest in [false, true] {
        let disk = disk();
        let mut s = open(&disk);
        put(&mut s, b"acked", b"value");
        s.checkpoint().unwrap();
        drop(s);
        for name in disk
            .list("wal")
            .unwrap()
            .into_iter()
            .filter(|n| n.ends_with(".seg"))
        {
            disk.clone().remove(&format!("wal/{name}")).unwrap();
        }
        for path in checkpoint_files(&disk, if keep_manifest { ".chunk" } else { ".manifest" }) {
            disk.clone().remove(&path).unwrap();
        }
        if keep_manifest {
            let path = checkpoint_files(&disk, ".manifest").remove(0);
            replace(&disk, &path, b"damaged");
        }
        assert!(Kernel::open(disk.clone(), clock(), SeededRng::new(3), &config()).is_err());
    }
}

#[test]
fn through_zero_orphan_on_an_empty_database_does_not_claim_nonempty_history() {
    let disk = disk();
    let mut s = open(&disk);
    s.checkpoint().unwrap();
    drop(s);
    for name in disk
        .list("wal")
        .unwrap()
        .into_iter()
        .filter(|n| n.ends_with(".seg"))
    {
        disk.clone().remove(&format!("wal/{name}")).unwrap();
    }
    let s = open(&disk);
    assert_eq!(s.snapshot().as_of, 0);
    assert_eq!(s.recovery_report().1.through_lsn, 0);
}

#[test]
fn legacy_numeric_segment_names_are_included_in_prefix_inventory() {
    let disk = disk();
    let mut s = open(&disk);
    let lsn = put(&mut s, b"acked", b"value");
    drop(s);
    let canonical = "wal/00000000000000000001.seg";
    let bytes = disk.read(canonical).unwrap();
    let mut d = disk.clone();
    d.append("wal/1.seg", &bytes).unwrap();
    d.sync("wal/1.seg").unwrap();
    d.remove(canonical).unwrap();
    d.sync_dir("wal").unwrap();
    let mut s = open(&disk);
    s.checkpoint().unwrap();
    drop(s);
    let s = open(&disk);
    assert_eq!(s.recovery_report().1.through_lsn, lsn);
    drop(s);
    d.remove("wal/1.seg").unwrap();
    assert!(RowStore::open(disk.clone(), clock(), SeededRng::new(8), &config()).is_err());
}

#[test]
fn replacing_the_entire_wal_with_a_valid_empty_segment_does_not_escape_source_evidence() {
    let disk = disk();
    let mut s = open(&disk);
    put(&mut s, b"acked", b"value");
    s.checkpoint().unwrap();
    drop(s);
    for name in disk
        .list("wal")
        .unwrap()
        .into_iter()
        .filter(|n| n.ends_with(".seg"))
    {
        disk.clone().remove(&format!("wal/{name}")).unwrap();
    }
    disk.clone()
        .append(
            "wal/00000000000000000001.seg",
            &lattice_core::record::segment_header(),
        )
        .unwrap();
    assert!(Kernel::open(disk.clone(), clock(), SeededRng::new(4), &config()).is_err());
}

#[test]
fn refusing_a_partial_header_below_the_source_boundary_preserves_the_damaged_bytes() {
    let disk = disk();
    let mut s = open(&disk);
    put(&mut s, b"acked", b"value");
    s.checkpoint().unwrap();
    drop(s);
    for name in disk
        .list("wal")
        .unwrap()
        .into_iter()
        .filter(|n| n.ends_with(".seg"))
    {
        disk.clone().remove(&format!("wal/{name}")).unwrap();
    }
    let path = "wal/00000000000000000001.seg";
    disk.clone().append(path, b"LAT").unwrap();
    assert!(Kernel::open(disk.clone(), clock(), SeededRng::new(4), &config()).is_err());
    assert_eq!(disk.read(path).unwrap(), b"LAT");
}

fn install_golden(case: &str) -> SharedIo {
    macro_rules! files {
        ($case:literal) => {
            [
                include_bytes!(concat!(
                    "../../../formats/corpus/checkpoint-v1/",
                    $case,
                    "/prefix.golden"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "../../../formats/corpus/checkpoint-v1/",
                    $case,
                    "/chunk-0000.golden"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "../../../formats/corpus/checkpoint-v1/",
                    $case,
                    "/manifest.golden"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "../../../formats/corpus/checkpoint-v1/",
                    $case,
                    "/seal-segment.golden"
                ))
                .as_slice(),
            ]
        };
    }
    let files = match case {
        "row" => files!("row"),
        "catalog" => files!("catalog"),
        _ => panic!("unknown fixture"),
    };
    let disk = disk();
    let mut d = disk.clone();
    d.create_dir_all("wal/.checkpoint").unwrap();
    let base = "wal/.checkpoint/00000000000000000005-0123456789abcdef0123456789abcdef";
    for (path, bytes) in [
        ("wal/00000000000000000001.seg".into(), files[0]),
        (format!("{base}.0000.chunk"), files[1]),
        (format!("{base}.manifest"), files[2]),
        ("wal/00000000000000000005.seg".into(), files[3]),
    ] {
        d.append(&path, bytes).unwrap();
        d.sync(&path).unwrap();
    }
    d.sync_dir("wal/.checkpoint").unwrap();
    d.sync_dir("wal").unwrap();
    d.sync_dir("").unwrap();
    disk
}
#[test]
fn legacy_golden_tags_fall_back_without_losing_rows_or_catalog_history() {
    let disk = install_golden("row");
    let store = RowStore::open_with_indexes(
        disk.clone(),
        clock(),
        SeededRng::new(7),
        &config(),
        vec![IndexDef {
            id: 7,
            table: 7,
            extract: |v| Some(v.to_vec()),
        }],
    )
    .unwrap();
    assert_eq!(store.recovery_report().1.through_lsn, 0);
    // The old tag lacks the complete typed catalog state; do not reinterpret its prefix.
    assert_eq!(
        store.get_at(Snapshot { as_of: 2 }, 7, b"k"),
        Some(&b"one".to_vec())
    );
    assert_eq!(
        store.get_at(Snapshot { as_of: 3 }, 7, b"k"),
        Some(&b"two".to_vec())
    );
    assert_eq!(store.get_at(Snapshot { as_of: 4 }, 7, b"k"), None);
    assert_eq!(
        store.get_at(Snapshot { as_of: 4 }, 7, b""),
        Some(&b"zero".to_vec())
    );
    for as_of in [2, 3, 4] {
        store.check_indexes_at(Snapshot { as_of }).unwrap();
    }
    drop(store);
    let (k, _) = Kernel::open(disk.clone(), clock(), SeededRng::new(8), &config()).unwrap();
    assert!(k.scrub().unwrap().issues.is_empty());
    drop(k);
    let disk = install_golden("catalog");
    let store = CatalogStore::open(disk.clone(), clock(), SeededRng::new(9), &config()).unwrap();
    assert_eq!(store.recovery_report().1.through_lsn, 0);
    // The old tag lacks the complete typed catalog state; do not reinterpret its prefix.
    assert_eq!(store.catalog().id_of(2, "before"), Some(7));
    assert_eq!(store.catalog().id_of(3, "after"), Some(7));
    assert_eq!(store.catalog().id_of(4, "next"), Some(8));
    drop(store);
    let (k, _) = Kernel::open(disk.clone(), clock(), SeededRng::new(10), &config()).unwrap();
    assert!(k.scrub().unwrap().issues.is_empty());
}
#[test]
fn uppercase_generation_names_are_noncanonical_cache_candidates() {
    let disk = install_golden("row");
    fn original_tag_through(disk: SharedIo) -> u64 {
        let (kernel, _, _, evidence) = Kernel::open_checkpointed(
            disk,
            clock(),
            SeededRng::new(21),
            &config(),
            b"raw-row-history-v1",
            Vec::<u8>::new,
            |reader, _, _| {
                let mut state = vec![0; usize::try_from(reader.remaining()).unwrap()];
                reader.raw(&mut state)?;
                Ok(state)
            },
            |_, _| Ok(()),
        )
        .unwrap();
        drop(kernel);
        evidence.through_lsn
    }
    // Prove the fixture is accepted under its original tag before changing case, so a wrong-tag
    // fallback cannot make this grammar test pass vacuously for the combined engine.
    assert_eq!(original_tag_through(disk.clone()), 4);

    let mut d = disk.clone();
    for name in disk.list("wal/.checkpoint").unwrap() {
        let new = name.replace(
            "0123456789abcdef0123456789abcdef",
            "0123456789ABCDEF0123456789ABCDEF",
        );
        let old = format!("wal/.checkpoint/{name}");
        let bytes = d.read(&old).unwrap();
        d.append(&format!("wal/.checkpoint/{new}"), &bytes).unwrap();
        d.remove(&old).unwrap();
    }
    assert_eq!(original_tag_through(disk.clone()), 0);
    let s = RowStore::open(disk.clone(), clock(), SeededRng::new(11), &config()).unwrap();
    assert_eq!(s.recovery_report().1.through_lsn, 0);
    assert_eq!(
        s.get_at(Snapshot { as_of: 2 }, 7, b"k"),
        Some(&b"one".to_vec())
    );
}
