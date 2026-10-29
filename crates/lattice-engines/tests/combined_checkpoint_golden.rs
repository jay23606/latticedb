//! Independent RFC0003 databases: actual fast restoration and exact public-store re-publication.
#![cfg(feature = "row")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lattice_core::{ChangeSetId, Config, Kernel, TurnRef};
use lattice_engines::{
    catalog::{self, CatalogOp, CatalogStore},
    mvcc::Snapshot,
    store::RowStore,
    typed,
    typed_store::TypedStore,
};
use lattice_lsl::validate::Value;
use lattice_seams::{Clock, Io, Rng};
use lattice_sim::{FaultProfile, SimIo};
use lattice_testkit::scenario::SharedIo;
use std::collections::BTreeMap;

const SOURCE_GENERATION: &str = "0123456789abcdef0123456789abcdef";
// Independent Python generation uses Clock=4242, owner Rng call=1, then new_id r1=2/r2=3.
const SECOND_GENERATION: &str = "00000000109200030000000000000002";
struct FixtureClock;
impl Clock for FixtureClock {
    fn now_ms(&self) -> u64 {
        4242
    }
}
#[derive(Default)]
struct FixtureRng(u64);
impl Rng for FixtureRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }
}
type Typed = TypedStore<SharedIo, FixtureClock, FixtureRng>;
type Raw = RowStore<SharedIo, FixtureClock, FixtureRng>;
type Catalog = CatalogStore<SharedIo, FixtureClock, FixtureRng>;

struct Golden {
    through: u64,
    prefix: &'static [u8],
    chunk: &'static [u8],
    manifest: &'static [u8],
    seal: &'static [u8],
    next_manifest: &'static [u8],
    next_seal: &'static [u8],
}
macro_rules! fixture {
    ($case:literal, $through:expr) => {
        Golden {
            through: $through,
            prefix: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/prefix.golden"
            )),
            chunk: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/chunk-0000.golden"
            )),
            manifest: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/manifest.golden"
            )),
            seal: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/seal-segment.golden"
            )),
            next_manifest: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/reencoded-manifest.golden"
            )),
            next_seal: include_bytes!(concat!(
                "../../../formats/corpus/combined-checkpoint-v1/",
                $case,
                "/reencoded-seal-segment.golden"
            )),
        }
    };
}
fn config() -> Config {
    Config::default()
}
fn install(golden: &Golden) -> SharedIo {
    let disk = SharedIo::new(SimIo::new(1, FaultProfile::clean()));
    let mut writer = disk.clone();
    writer.create_dir_all("wal/.checkpoint").unwrap();
    let base = format!(
        "wal/.checkpoint/{:020}-{SOURCE_GENERATION}",
        golden.through + 1
    );
    for (path, bytes) in [
        ("wal/00000000000000000001.seg".to_string(), golden.prefix),
        (format!("{base}.0000.chunk"), golden.chunk),
        (format!("{base}.manifest"), golden.manifest),
        (format!("wal/{:020}.seg", golden.through + 1), golden.seal),
    ] {
        writer.append(&path, bytes).unwrap();
        writer.sync(&path).unwrap();
    }
    writer.sync_dir("wal/.checkpoint").unwrap();
    writer.sync_dir("wal").unwrap();
    writer.sync_dir("").unwrap();
    disk.crash();
    disk
}
fn typed_open(disk: &SharedIo) -> Typed {
    TypedStore::open(disk.clone(), FixtureClock, FixtureRng::default(), &config()).unwrap()
}
fn raw_open(disk: &SharedIo) -> Raw {
    RowStore::open(disk.clone(), FixtureClock, FixtureRng::default(), &config()).unwrap()
}
fn catalog_open(disk: &SharedIo) -> Catalog {
    CatalogStore::open(disk.clone(), FixtureClock, FixtureRng::default(), &config()).unwrap()
}
fn object(points: i64, aaa: Option<bool>) -> Value {
    let mut fields = BTreeMap::from([("points".to_string(), Value::Int(points))]);
    if let Some(value) = aaa {
        fields.insert("aaa".to_string(), Value::Bool(value));
    }
    Value::Object(fields)
}
fn assert_typed_values(store: &Typed) {
    assert_eq!(
        store
            .get_at(Snapshot { as_of: 3 }, "Ticket", b"old")
            .unwrap(),
        Some(object(3, None))
    );
    assert_eq!(
        store
            .get_at(Snapshot { as_of: 3 }, "Ticket", b"same")
            .unwrap(),
        Some(object(4, None))
    );
    assert_eq!(
        store
            .get_at(Snapshot { as_of: 3 }, "Ticket", b"new")
            .unwrap(),
        None
    );
    assert_eq!(
        store.get_at(store.snapshot(), "Ticket", b"old").unwrap(),
        Some(object(3, None))
    );
    assert_eq!(
        store.get_at(store.snapshot(), "Ticket", b"same").unwrap(),
        Some(object(9, Some(false)))
    );
    assert_eq!(
        store.get_at(store.snapshot(), "Ticket", b"new").unwrap(),
        Some(object(8, Some(true)))
    );
    assert_eq!(store.catalog().schema(1).unwrap().installed_lsn, 2);
    assert_eq!(store.catalog().schema(2).unwrap().installed_lsn, 4);
}
fn assert_second_container(disk: &SharedIo, golden: &Golden, seal: u64) {
    assert_eq!(seal, golden.through + 2);
    let base = format!("wal/.checkpoint/{seal:020}-{SECOND_GENERATION}");
    // This exercises the public store's actual complete encoder, including kernel/catalog/rows.
    assert_eq!(
        disk.read(&format!("{base}.0000.chunk")).unwrap(),
        golden.chunk
    );
    assert_eq!(
        disk.read(&format!("{base}.manifest")).unwrap(),
        golden.next_manifest
    );
    assert_eq!(
        disk.read(&format!("wal/{seal:020}.seg")).unwrap(),
        golden.next_seal
    );
    assert_eq!(
        disk.read("wal/00000000000000000001.seg").unwrap(),
        golden.prefix
    );
    assert_eq!(
        disk.read(&format!("wal/{:020}.seg", golden.through + 1))
            .unwrap(),
        golden.seal
    );
}
fn assert_full_kernel_agrees(disk: &SharedIo, golden: &Golden) {
    let (mut kernel, recovered) =
        Kernel::open(disk.clone(), FixtureClock, FixtureRng::default(), &config()).unwrap();
    assert_eq!(recovered.records.len() as u64, golden.through + 2);
    for id in 1..golden.through {
        assert!(kernel.contains(&ChangeSetId::from_bytes(u128::from(id).to_be_bytes())));
    }
    assert!(
        kernel
            .begin_turn(TurnRef {
                session: 9,
                turn: 1
            })
            .is_err()
    );
    assert!(kernel.scrub().unwrap().issues.is_empty());
}

#[test]
fn independent_evolved_rows_restore_fast_and_public_checkpoint_reproduces_exact_container() {
    let golden = fixture!("evolved-rows", 6);
    let disk = install(&golden);
    let mut store = typed_open(&disk);
    assert_eq!(
        store.recovery_report().1.through_lsn,
        6,
        "fallback must not masquerade as fast restore"
    );
    assert_eq!(
        store.recovery_report().1.skipped_wal_bytes,
        golden.prefix.len() as u64
    );
    assert_typed_values(&store);
    let seal = store.checkpoint().unwrap();
    assert_second_container(&disk, &golden, seal);
    drop(store);
    disk.crash();
    assert_full_kernel_agrees(&disk, &golden);
    let mut store = typed_open(&disk);
    assert_eq!(store.recovery_report().1.through_lsn, 7);
    assert_typed_values(&store);
    store
        .install_schema("type New { title: text } type Ticket { aaa: bool, points: int in 0..=10 }")
        .unwrap();
    assert_eq!(
        store.catalog().id_of(store.snapshot().as_of, "New"),
        Some(3),
        "raw table 2 tombstone reservation must survive both fast openings"
    );
}

#[test]
fn independent_invisible_schema_lifetime_restores_fast_and_public_catalog_checkpoint_is_exact() {
    let golden = fixture!("invisible-catalog", 3);
    let disk = install(&golden);
    let mut store = catalog_open(&disk);
    assert_eq!(
        store.recovery_report().1.through_lsn,
        3,
        "fallback must not hide a broken lifetime decoder"
    );
    assert_eq!(
        store.recovery_report().1.skipped_wal_bytes,
        golden.prefix.len() as u64
    );
    assert!(store.catalog().as_of(2).is_empty());
    assert_eq!(
        store.catalog().current(),
        BTreeMap::from([(8, "Other".to_string())])
    );
    assert_eq!(store.catalog().schema(1).unwrap().installed_lsn, 2);
    assert!(store.catalog().schema(1).unwrap().layouts.contains_key(&7));
    assert!(
        store
            .commit(vec![CatalogOp::Create {
                table: 7,
                name: "reuse".to_string()
            }])
            .is_err()
    );
    let seal = store.checkpoint().unwrap();
    assert_second_container(&disk, &golden, seal);
    drop(store);
    disk.crash();
    assert_full_kernel_agrees(&disk, &golden);
    let store = catalog_open(&disk);
    assert_eq!(store.recovery_report().1.through_lsn, 4);
    assert_eq!(store.catalog().schema(1).unwrap().installed_lsn, 2);
    assert_eq!(store.catalog().next_table_id().unwrap(), 9);
}

#[test]
fn independent_lcat_and_original_layout_cells_reencode_exactly_and_public_put_matches_ltyp() {
    for bytes in [
        include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/schema-v1.golden").as_slice(),
        include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/schema-v2.golden").as_slice(),
        include_bytes!("../../../formats/corpus/combined-checkpoint-v1/invisible-catalog/atomic-schema-drop.golden").as_slice(),
    ] { assert_eq!(catalog::encode(&catalog::decode(bytes).unwrap()).unwrap(),bytes); }
    let golden = fixture!("evolved-rows", 6);
    let disk = install(&golden);
    let raw = raw_open(&disk);
    assert_eq!(raw.recovery_report().1.through_lsn, 6);
    let cases=[
        (1,3,b"old".as_slice(),include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/envelope-v1-old.golden").as_slice()),
        (1,3,b"same".as_slice(),include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/envelope-v1-same.golden").as_slice()),
        (2,5,b"new".as_slice(),include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/envelope-v2-new.golden").as_slice()),
        (2,5,b"same".as_slice(),include_bytes!("../../../formats/corpus/combined-checkpoint-v1/evolved-rows/envelope-v2-same.golden").as_slice()),
    ];
    for (version, lsn, key, expected) in cases {
        assert_eq!(
            raw.get_at(Snapshot { as_of: lsn }, 1, key)
                .unwrap()
                .as_slice(),
            expected
        );
        let layout = raw
            .catalog()
            .schema(version)
            .unwrap()
            .layouts
            .get(&1)
            .unwrap();
        let cells = expected.get(13..).unwrap();
        assert_eq!(
            typed::encode_row(layout, &typed::decode_row(layout, cells).unwrap()).unwrap(),
            cells
        );
    }
    assert!(raw.has_table_history(2));
    drop(raw);
    let mut store = typed_open(&disk);
    assert_eq!(store.recovery_report().1.through_lsn, 6);
    let mut tx = store.begin();
    store
        .put(&mut tx, "Ticket", b"generated", &object(8, Some(true)))
        .unwrap();
    store.commit(tx).unwrap();
    drop(store);
    let raw = raw_open(&disk);
    assert_eq!(raw.recovery_report().1.through_lsn, 6);
    assert_eq!(
        raw.get_at(raw.snapshot(), 1, b"generated")
            .unwrap()
            .as_slice(),
        include_bytes!(
            "../../../formats/corpus/combined-checkpoint-v1/evolved-rows/envelope-v2-new.golden"
        )
    );
}
