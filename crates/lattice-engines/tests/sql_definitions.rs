//! Durable SQL table definitions in the engine (RFC 0012, ADR 0028, issue 572): the LCIX version 3
//! envelope, capability bit 3 and old-reader refusal, replay validation, the catalog projection,
//! the explicit format-upgrade setting and a power-cut sweep over table creation.
//!
//! Expected values are written by hand or taken from the independent Python vectors in
//! `formats/corpus/catalog-index-v3-*`; none is derived by running the code under test.
#![cfg(feature = "row")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use lattice_core::admission::{MANAGED_COMPOUND, decode_header};
use lattice_core::bugs::{self, Bug};
use lattice_core::{ChangeSet, Config, Kernel};
use lattice_engines::catalog::{
    self, CatalogError, CatalogOp, CatalogStore, CompoundChange, StoreError,
};
use lattice_engines::limits::AllowAll;
use lattice_engines::sqldef::{
    Column, DefaultValue, DefineTable, KeyConstraint, SqlDefError, SqlTableDef, SqlType,
};
use lattice_engines::store::{RowError, RowStore};
use lattice_seams::Io;
use lattice_sim::{FaultProfile, SeededRng, SimClock, SimIo};
use lattice_testkit::scenario::{SharedClock, SharedIo};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Required capabilities 1 and 8 written as a literal, not taken from the code under test.
const BITS_0_AND_3: u64 = 9;

type Store = RowStore<SharedIo, SharedClock, SeededRng>;

fn config() -> Config {
    Config {
        segment_limit: 4_096,
        ..Config::default()
    }
}

fn fresh() -> (SharedIo, Rc<SimClock>) {
    (
        SharedIo::new(SimIo::new(17, FaultProfile::clean())),
        Rc::new(SimClock::new(1_000)),
    )
}

fn try_open(disk: &SharedIo, clock: &Rc<SimClock>, config: &Config) -> Result<Store, RowError> {
    // A fresh timestamp per open keeps a reseeded generator from reusing change set ids.
    clock.advance(1);
    RowStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        config,
    )
}

fn open(disk: &SharedIo, clock: &Rc<SimClock>) -> Store {
    try_open(disk, clock, &config()).unwrap()
}

fn create(id: u32) -> CatalogOp {
    CatalogOp::Create {
        table: id,
        name: format!("t{id}"),
    }
}

fn column(name: &str, ty: SqlType, not_null: bool) -> Column {
    Column {
        name: name.into(),
        ty,
        varchar_limit: 0,
        not_null,
        default: None,
    }
}

/// `id integer not null primary key, title varchar(20) default 'x' unique`.
fn items_def() -> SqlTableDef {
    SqlTableDef {
        columns: vec![
            column("id", SqlType::Integer, true),
            Column {
                name: "title".into(),
                ty: SqlType::Varchar,
                varchar_limit: 20,
                not_null: false,
                default: Some(DefaultValue::Text("x".into())),
            },
        ],
        primary_key: Some(KeyConstraint {
            columns: vec![0],
            name: "items_pkey".into(),
        }),
        uniques: vec![KeyConstraint {
            columns: vec![1],
            name: "items_title_key".into(),
        }],
    }
}

/// One nullable integer column named `id`, no keys.
fn small_def() -> SqlTableDef {
    SqlTableDef {
        columns: vec![column("id", SqlType::Integer, false)],
        primary_key: None,
        uniques: Vec::new(),
    }
}

fn define(id: u32, definition: SqlTableDef) -> DefineTable {
    DefineTable {
        table_id: id,
        definition,
    }
}

fn wal(disk: &SharedIo) -> Vec<(String, Vec<u8>)> {
    disk.list("wal")
        .unwrap()
        .into_iter()
        .filter(|p| p.ends_with(".seg"))
        .map(|p| {
            let bytes = disk.read(&format!("wal/{p}")).unwrap();
            (p, bytes)
        })
        .collect()
}

/// The required-capability mask of every segment header, in segment order, read from the bytes.
fn masks(disk: &SharedIo) -> Vec<u64> {
    wal(disk)
        .iter()
        .map(|(_, bytes)| decode_header(bytes).unwrap().required_capabilities)
        .collect()
}

fn rows(store: &Store, table: u32) -> BTreeMap<Vec<u8>, Vec<u8>> {
    store
        .scan_at(store.snapshot(), table, .., &AllowAll)
        .unwrap()
        .into_iter()
        .map(|(key, value)| (key.to_vec(), value.clone()))
        .collect()
}

/// Appends an arbitrary payload through the kernel, bypassing every engine-side check, to build a
/// history a correct writer would never produce.
fn append_raw(disk: &SharedIo, clock: &Rc<SimClock>, payload: Vec<u8>, required: u64) {
    clock.advance(1);
    let (mut kernel, _, _) = Kernel::open_streaming_with_capabilities(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        &config(),
        BITS_0_AND_3,
        |_, _| {},
    )
    .unwrap();
    let id = kernel.new_id();
    kernel
        .commit_with_capabilities(ChangeSet::new(id, payload), required)
        .unwrap();
}

/// Both recovery consumers must refuse with a corruption error naming `needle`.
fn assert_replay_refused(disk: &SharedIo, clock: &Rc<SimClock>, needle: &str) {
    let before = wal(disk);
    match try_open(disk, clock, &config()) {
        Err(RowError::Corrupt(message)) => {
            assert!(message.contains(needle), "row store said: {message}")
        }
        Err(other) => panic!("row store: expected corruption, got {other:?}"),
        Ok(_) => panic!("row store accepted the history"),
    }
    clock.advance(1);
    match CatalogStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        &config(),
    ) {
        Err(StoreError::Corrupt(message)) => {
            assert!(message.contains(needle), "catalog store said: {message}")
        }
        Err(other) => panic!("catalog store: expected corruption, got {other:?}"),
        Ok(_) => panic!("catalog store accepted the history"),
    }
    assert_eq!(
        wal(disk),
        before,
        "a refused replay must not repair or write"
    );
}

struct BugGuard;
impl Drop for BugGuard {
    fn drop(&mut self) {
        bugs::clear();
    }
}

// ---------------------------------------------------------------------------------------------
// old-reader refusal and the guard

#[test]
fn old_reader_refuses_a_directory_with_sql_definitions() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    store
        .commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, items_def())],
        )
        .unwrap();
    drop(store);
    // Literal oracle: one guarded segment whose header says capabilities 1 and 8 (mask 9).
    let segments = wal(&disk);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].1.get(..8), Some(b"LATCAP02".as_slice()));
    assert_eq!(masks(&disk), vec![9]);

    let before = wal(&disk);
    // A reader that supports nothing, one that supports only the managed compound (every build
    // before this change), and callback mode all refuse before a record is read or repaired.
    assert!(lattice_core::Log::open(disk.clone(), "wal", 4_096).is_err());
    let mut visited = false;
    let managed_only = Kernel::open_streaming_with_capabilities(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        &config(),
        MANAGED_COMPOUND,
        |_, _| visited = true,
    );
    assert!(managed_only.is_err());
    assert!(!visited, "the guard must refuse before any callback");
    let callback = RowStore::open_with_indexes(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        &config(),
        vec![lattice_engines::index::IndexDef {
            id: 1,
            table: 7,
            extract: |_| panic!("the guard must refuse before a callback runs"),
        }],
    );
    assert!(matches!(callback, Err(RowError::Rejected(_))));
    assert_eq!(wal(&disk), before, "refusal must not change a byte");

    // The build that lists bit 3 recovers the definition.
    let store = open(&disk, &clock);
    assert_eq!(store.catalog().definition(7), Some(&items_def()));
}

#[test]
fn a_change_without_definitions_never_requests_bit_3() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    store.commit_catalog(vec![create(7)], Vec::new()).unwrap();
    drop(store);
    assert_eq!(masks(&disk), vec![1], "managed compound only");
    // The same bytes as before this change: version 1 envelope.
    let v1 = catalog::encode_compound(&CompoundChange {
        tables: vec![create(7)],
        indexes: Vec::new(),
        definitions: Vec::new(),
        rows: Vec::new(),
    })
    .unwrap();
    assert_eq!(v1.get(..5), Some(b"LCIX\x01".as_slice()));
}

#[test]
fn the_missing_guard_bit_is_caught_by_the_header_oracle_and_by_recovery() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        if mutated {
            bugs::enable(Bug::GuardBitNotRequested);
        }
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(7)],
                Vec::new(),
                vec![define(7, small_def())],
            )
            .unwrap();
        drop(guard);
        drop(store);
        if mutated {
            assert_eq!(masks(&disk), vec![1], "the planted bug leaves bit 3 out");
            assert_replay_refused(&disk, &clock, "without capability 8");
        } else {
            assert_eq!(masks(&disk), vec![9]);
            assert_eq!(
                open(&disk, &clock).catalog().definition(7),
                Some(&small_def())
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// replay validation

/// A version 1 payload turned into a version 3 payload that carries no definition: the encoder
/// never writes one, so the bytes are framed by hand (version byte, then an empty definition count
/// before the row block).
fn v3_without_definitions() -> Vec<u8> {
    let v1 = catalog::encode_compound(&CompoundChange {
        tables: vec![create(7)],
        indexes: Vec::new(),
        definitions: Vec::new(),
        rows: Vec::new(),
    })
    .unwrap();
    let mut v3 = v1[..v1.len() - 4].to_vec();
    v3[4] = 3;
    v3.extend_from_slice(&0u32.to_le_bytes());
    v3.extend_from_slice(&0u32.to_le_bytes());
    v3
}

fn payload(tables: Vec<CatalogOp>, definitions: Vec<DefineTable>) -> Vec<u8> {
    catalog::encode_compound(&CompoundChange {
        tables,
        indexes: Vec::new(),
        definitions,
        rows: Vec::new(),
    })
    .unwrap()
}

#[test]
fn create_without_definition_and_definition_without_create_are_refused_at_replay() {
    // A version 3 envelope must carry at least one definition: a create written as version 3
    // with an empty definition block is refused, not read as a plain create.
    let (disk, clock) = fresh();
    append_raw(&disk, &clock, v3_without_definitions(), BITS_0_AND_3);
    assert_replay_refused(&disk, &clock, "EmptyDefinitions");

    // A definition for a table that no catalog operation created is refused.
    let (disk, clock) = fresh();
    append_raw(
        &disk,
        &clock,
        payload(Vec::new(), vec![define(7, small_def())]),
        BITS_0_AND_3,
    );
    assert_replay_refused(&disk, &clock, "NoSuchTable(7)");

    // A definition for a table that the same change drops is refused too.
    let (disk, clock) = fresh();
    append_raw(
        &disk,
        &clock,
        payload(vec![create(7)], Vec::new()),
        MANAGED_COMPOUND,
    );
    append_raw(
        &disk,
        &clock,
        payload(
            vec![CatalogOp::Drop { table: 7 }],
            vec![define(7, small_def())],
        ),
        BITS_0_AND_3,
    );
    assert_replay_refused(&disk, &clock, "NoSuchTable(7)");

    // A version 3 payload in a segment that lacks bit 3 (or bit 0) is refused as unguarded.
    let (disk, clock) = fresh();
    append_raw(
        &disk,
        &clock,
        payload(vec![create(7)], vec![define(7, small_def())]),
        MANAGED_COMPOUND,
    );
    assert_replay_refused(&disk, &clock, "without capability 8");
}

#[test]
fn second_definition_for_a_table_is_refused() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    store
        .commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, items_def())],
        )
        .unwrap();
    let before = wal(&disk);
    let again = store.commit_with_definitions(
        store.begin(),
        Vec::new(),
        Vec::new(),
        vec![define(7, small_def())],
    );
    assert_eq!(
        again,
        Err(RowError::InvalidCatalog(CatalogError::AlreadyDefined(7)))
    );
    // Two definitions for one new table in one change set.
    let twice = store.commit_with_definitions(
        store.begin(),
        vec![create(8)],
        Vec::new(),
        vec![define(8, small_def()), define(8, small_def())],
    );
    assert_eq!(
        twice,
        Err(RowError::InvalidCatalog(CatalogError::AlreadyDefined(8)))
    );
    // Not ascending: the envelope cannot carry it.
    let unsorted = store.commit_with_definitions(
        store.begin(),
        vec![create(8), create(9)],
        Vec::new(),
        vec![define(9, small_def()), define(8, small_def())],
    );
    assert_eq!(
        unsorted,
        Err(RowError::InvalidCatalog(CatalogError::BadDefinition(
            SqlDefError::NotAscending
        )))
    );
    assert_eq!(wal(&disk), before, "a refused definition appends nothing");
    assert_eq!(store.catalog().definition(7), Some(&items_def()));
    drop(store);

    // The same history written by a faulty writer is refused at replay and recovery stops.
    append_raw(
        &disk,
        &clock,
        payload(Vec::new(), vec![define(7, small_def())]),
        BITS_0_AND_3,
    );
    assert_replay_refused(&disk, &clock, "AlreadyDefined(7)");
}

#[test]
fn an_invalid_definition_is_refused_before_anything_is_written() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    let before = wal(&disk);
    let mut bad = small_def();
    bad.columns.push(column("id", SqlType::Text, false));
    let refused = store.commit_with_definitions(
        store.begin(),
        vec![create(7)],
        Vec::new(),
        vec![define(7, bad)],
    );
    assert_eq!(
        refused,
        Err(RowError::InvalidCatalog(CatalogError::BadDefinition(
            SqlDefError::DuplicateColumn
        )))
    );
    let big = store.commit_with_definitions(
        store.begin(),
        vec![create(1 << 26)],
        Vec::new(),
        vec![define(1 << 26, small_def())],
    );
    assert_eq!(
        big,
        Err(RowError::InvalidCatalog(CatalogError::BadDefinition(
            SqlDefError::BadTableId(1 << 26)
        )))
    );
    assert_eq!(wal(&disk), before);
}

// ---------------------------------------------------------------------------------------------
// catalog projection

#[test]
fn catalog_store_projects_definitions_and_applies_no_rows() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    let mut tx = store.begin();
    tx.put((7, b"a".to_vec()), b"row".to_vec());
    let created = store
        .commit_with_definitions(
            tx,
            vec![create(7), create(8)],
            Vec::new(),
            vec![define(7, items_def()), define(8, small_def())],
        )
        .unwrap();
    store.commit_catalog(vec![create(9)], Vec::new()).unwrap();
    let dropped = store
        .commit_catalog(vec![CatalogOp::Drop { table: 8 }], Vec::new())
        .unwrap();
    drop(store);

    clock.advance(1);
    let projection = CatalogStore::open(
        disk.clone(),
        SharedClock(clock.clone()),
        SeededRng::new(17),
        &config(),
    )
    .unwrap();
    let catalog = projection.catalog();
    assert_eq!(catalog.definition(7), Some(&items_def()));
    assert_eq!(
        catalog.definition(8),
        None,
        "dropped from the current catalog"
    );
    assert_eq!(catalog.definition(9), None, "a raw table stays undefined");
    assert_eq!(
        catalog.definitions().keys().copied().collect::<Vec<_>>(),
        vec![7]
    );
    assert_eq!(catalog.defined_at(7), Some(created));
    // The dropped table's definition still decodes snapshots from before the drop.
    assert_eq!(catalog.definition_as_of(8, created), Some(&small_def()));
    assert_eq!(catalog.definition_as_of(8, dropped - 1), Some(&small_def()));
    assert_eq!(catalog.definition_as_of(8, dropped), None);
    assert_eq!(catalog.definition_as_of(8, created - 1), None);
    assert_eq!(catalog.definition_as_of(7, created - 1), None);
    assert_eq!(catalog.definition_as_of(7, u64::MAX), Some(&items_def()));
}

#[test]
fn definitions_survive_reopen_rename_and_drop_with_as_of_history() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    let created = store
        .commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, items_def())],
        )
        .unwrap();
    let renamed = store
        .commit_catalog(
            vec![CatalogOp::Rename {
                table: 7,
                name: "renamed".into(),
            }],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(store.catalog().definition(7), Some(&items_def()));
    let dropped = store
        .commit_catalog(vec![CatalogOp::Drop { table: 7 }], Vec::new())
        .unwrap();
    assert_eq!(store.catalog().definition(7), None);
    drop(store);

    disk.crash();
    let store = open(&disk, &clock);
    let catalog = store.catalog();
    assert_eq!(catalog.definition(7), None);
    assert_eq!(catalog.definition_as_of(7, created), Some(&items_def()));
    assert_eq!(catalog.definition_as_of(7, renamed), Some(&items_def()));
    assert_eq!(catalog.definition_as_of(7, dropped - 1), Some(&items_def()));
    assert_eq!(catalog.definition_as_of(7, dropped), None);
    drop(store);
    // A dropped id is never reused, so its definition cannot be replaced.
    let mut store = open(&disk, &clock);
    let again = store.commit_with_definitions(
        store.begin(),
        vec![create(7)],
        Vec::new(),
        vec![define(7, small_def())],
    );
    assert_eq!(
        again,
        Err(RowError::InvalidCatalog(CatalogError::IdUsed(7)))
    );
}

#[test]
fn defining_an_existing_table_is_refused_until_adoption_exists() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    store.commit_catalog(vec![create(7)], Vec::new()).unwrap();
    store.set_allow_format_upgrade(true);
    let before = wal(&disk);
    // The bypass shape: a definition for a table this change does not create.
    for tables in [Vec::new(), vec![create(8)]] {
        assert_eq!(
            store.commit_with_definitions(
                store.begin(),
                tables,
                Vec::new(),
                vec![define(7, small_def())],
            ),
            Err(RowError::InvalidCatalog(
                CatalogError::DefinitionNotInCreate(7)
            ))
        );
    }
    assert_eq!(wal(&disk), before, "a refused definition appends nothing");
    assert_eq!(store.catalog().definition(7), None);
    // A same-change create and definition is allowed.
    store
        .commit_with_definitions(
            store.begin(),
            vec![create(9)],
            Vec::new(),
            vec![define(9, small_def())],
        )
        .unwrap();
    drop(store);
    // A faulty writer's definition-only change for an existing table is refused at replay too.
    append_raw(
        &disk,
        &clock,
        payload(Vec::new(), vec![define(7, small_def())]),
        BITS_0_AND_3,
    );
    assert_replay_refused(&disk, &clock, "DefinitionNotInCreate(7)");
}

#[test]
fn adoption_of_an_existing_table_is_caught_by_the_catalog_oracle() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        store.commit_catalog(vec![create(7)], Vec::new()).unwrap();
        store.set_allow_format_upgrade(true);
        if mutated {
            bugs::enable(Bug::DefinitionOfExistingTableAccepted);
        }
        let result = store.commit_with_definitions(
            store.begin(),
            Vec::new(),
            Vec::new(),
            vec![define(7, small_def())],
        );
        drop(guard);
        if mutated {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn a_dropped_definition_is_caught_on_replay_and_in_the_live_catalog() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(7)],
                Vec::new(),
                vec![define(7, items_def())],
            )
            .unwrap();
        if mutated {
            bugs::enable(Bug::DefinitionDropIgnored);
        }
        store
            .commit_catalog(vec![CatalogOp::Drop { table: 7 }], Vec::new())
            .unwrap();
        let live = store.catalog().definition(7).cloned();
        drop(store);
        drop(guard);
        // Independent expectation: a dropped table has no live definition.
        assert_eq!(live.is_some(), mutated);
        assert_eq!(open(&disk, &clock).catalog().definition(7), None);
    }
}

#[test]
fn a_definition_lost_at_replay_fails_the_reopen_oracle() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(7)],
                Vec::new(),
                vec![define(7, items_def())],
            )
            .unwrap();
        drop(store);
        if mutated {
            bugs::enable(Bug::DefinitionDroppedOnReplay);
        }
        let reopened = open(&disk, &clock);
        let recovered = reopened.catalog().definition(7).cloned();
        let table_back = reopened.catalog().current().contains_key(&7);
        drop(reopened);
        let projected = CatalogStore::open(
            disk.clone(),
            SharedClock(clock.clone()),
            SeededRng::new(17),
            &config(),
        )
        .unwrap();
        drop(guard);
        let expected = if mutated { None } else { Some(items_def()) };
        assert_eq!(recovered, expected);
        assert_eq!(projected.catalog().definition(7).cloned(), expected);
        assert!(table_back, "the table itself is recovered either way");
    }
}

#[test]
fn a_second_definition_or_one_without_a_table_is_caught_by_the_catalog_oracle() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(7)],
                Vec::new(),
                vec![define(7, items_def())],
            )
            .unwrap();
        if mutated {
            // A second definition can only arrive as a definition-only change, which the
            // create-only rule also refuses, so both defects are needed to reach the check.
            bugs::enable(Bug::SecondDefinitionReplaces);
            bugs::enable(Bug::DefinitionOfExistingTableAccepted);
        }
        let second = store.commit_with_definitions(
            store.begin(),
            Vec::new(),
            Vec::new(),
            vec![define(7, small_def())],
        );
        drop(guard);
        bugs::clear();
        if mutated {
            assert!(
                second.is_ok(),
                "the planted bug lets the second definition in"
            );
            assert_eq!(store.catalog().definition(7), Some(&small_def()));
        } else {
            assert_eq!(
                second,
                Err(RowError::InvalidCatalog(CatalogError::AlreadyDefined(7)))
            );
            assert_eq!(store.catalog().definition(7), Some(&items_def()));
        }

        let guard = BugGuard;
        if mutated {
            bugs::enable(Bug::DefinitionForMissingTableAccepted);
            bugs::enable(Bug::DefinitionOfExistingTableAccepted);
        }
        let missing = store.commit_with_definitions(
            store.begin(),
            Vec::new(),
            Vec::new(),
            vec![define(99, small_def())],
        );
        drop(guard);
        if mutated {
            assert!(
                missing.is_ok(),
                "the planted bug accepts a definition without a table"
            );
        } else {
            assert_eq!(
                missing,
                Err(RowError::InvalidCatalog(CatalogError::NoSuchTable(99)))
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// explicit format upgrade

#[test]
fn the_first_definition_in_an_existing_directory_needs_the_upgrade_setting() {
    for prefix in ["rows", "managed"] {
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        if prefix == "rows" {
            let mut tx = store.begin();
            tx.put((5, b"p".to_vec()), b"1".to_vec());
            store.commit(tx).unwrap();
        } else {
            store.commit_catalog(vec![create(5)], Vec::new()).unwrap();
        }
        assert!(!store.allow_format_upgrade());
        let before = wal(&disk);
        let applied_before = store.applied_lsn();
        let refused = store.commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, items_def())],
        );
        assert_eq!(refused, Err(RowError::FormatUpgradeRequired), "{prefix}");
        let message = RowError::FormatUpgradeRequired.to_string();
        assert!(message.contains("allow-format-upgrade") && message.contains("backup"));
        assert_eq!(
            wal(&disk),
            before,
            "refusal writes nothing and installs no guard"
        );
        assert_eq!(store.applied_lsn(), applied_before);
        assert_eq!(store.catalog().definition(7), None);
        // Reads and ordinary writes are unaffected, and a change without a definition still works.
        store.commit_catalog(vec![create(8)], Vec::new()).unwrap();

        store.set_allow_format_upgrade(true);
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(7)],
                Vec::new(),
                vec![define(7, items_def())],
            )
            .unwrap();
        // The legacy prefix keeps its old header; the segment that holds the definition is guarded.
        let after = masks(&disk);
        assert_eq!(after.last(), Some(&9), "{prefix}: {after:?}");
        // After the guard is installed the setting is no longer consulted.
        store.set_allow_format_upgrade(false);
        store
            .commit_with_definitions(
                store.begin(),
                vec![create(9)],
                Vec::new(),
                vec![define(9, small_def())],
            )
            .unwrap();
        drop(store);
        let store = open(&disk, &clock);
        assert_eq!(store.catalog().definition(7), Some(&items_def()));
        assert_eq!(store.catalog().definition(9), Some(&small_def()));
        assert!(
            !store.allow_format_upgrade(),
            "the setting is per open, never persisted"
        );
    }
}

#[test]
fn a_new_empty_directory_needs_no_upgrade_setting() {
    let (disk, clock) = fresh();
    let mut store = open(&disk, &clock);
    assert!(!store.allow_format_upgrade());
    store
        .commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, items_def())],
        )
        .unwrap();
}

#[test]
fn skipping_the_upgrade_check_is_caught() {
    for mutated in [false, true] {
        bugs::clear();
        let guard = BugGuard;
        let (disk, clock) = fresh();
        let mut store = open(&disk, &clock);
        store.commit_catalog(vec![create(5)], Vec::new()).unwrap();
        if mutated {
            bugs::enable(Bug::FormatUpgradeUnchecked);
        }
        let result = store.commit_with_definitions(
            store.begin(),
            vec![create(7)],
            Vec::new(),
            vec![define(7, small_def())],
        );
        drop(guard);
        if mutated {
            assert!(
                result.is_ok(),
                "the planted bug upgrades without the setting"
            );
            assert_eq!(masks(&disk).last(), Some(&9));
        } else {
            assert_eq!(result, Err(RowError::FormatUpgradeRequired));
            assert_eq!(masks(&disk), vec![1]);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// independent LCIX version 3 vectors (formats/generate_sql_definition_golden.py)

const CREATE_AND_DEFINITION: &[u8] =
    include_bytes!("../../../formats/corpus/catalog-index-v3-ok-create-and-definition.golden");
const CREATE_DEFINITION_AND_ROWS: &[u8] =
    include_bytes!("../../../formats/corpus/catalog-index-v3-ok-create-definition-and-rows.golden");
const DEFINITION_ONLY: &[u8] =
    include_bytes!("../../../formats/corpus/catalog-index-v3-ok-definition-only.golden");

/// The composite definition of the vector, written out field by field.
fn composite_def() -> SqlTableDef {
    SqlTableDef {
        columns: vec![
            column("tenant", SqlType::Uuid, true),
            column("id", SqlType::BigInt, true),
            column("title", SqlType::Text, false),
        ],
        primary_key: Some(KeyConstraint {
            columns: vec![0, 1],
            name: "items_pkey".into(),
        }),
        uniques: vec![KeyConstraint {
            columns: vec![2],
            name: "items_title_key".into(),
        }],
    }
}

fn items_create() -> CatalogOp {
    CatalogOp::Create {
        table: 7,
        name: "items".into(),
    }
}

#[test]
fn independent_lcix_v3_vectors_decode_and_re_encode_byte_for_byte() {
    let cases: [(&[u8], CompoundChange); 3] = [
        (
            CREATE_AND_DEFINITION,
            CompoundChange {
                tables: vec![items_create()],
                indexes: Vec::new(),
                definitions: vec![define(7, small_def())],
                rows: Vec::new(),
            },
        ),
        (
            CREATE_DEFINITION_AND_ROWS,
            CompoundChange {
                tables: vec![items_create()],
                indexes: Vec::new(),
                definitions: vec![define(7, small_def())],
                rows: vec![((7, b"a".to_vec()), Some(b"x".to_vec()))],
            },
        ),
        (
            DEFINITION_ONLY,
            CompoundChange {
                tables: Vec::new(),
                indexes: Vec::new(),
                definitions: vec![define(5, small_def()), define(9, composite_def())],
                rows: Vec::new(),
            },
        ),
    ];
    for (bytes, expected) in cases {
        assert_eq!(catalog::decode_compound(bytes), Ok(expected.clone()));
        assert_eq!(catalog::encode_compound(&expected).unwrap(), bytes);
        assert_eq!(bytes.get(..5), Some(b"LCIX\x03".as_slice()));
        for cut in 0..bytes.len() {
            assert!(
                catalog::decode_compound(&bytes[..cut]).is_err(),
                "strict prefix {cut} was accepted"
            );
        }
        // Every single-bit change is refused or re-encodes to exactly the changed bytes.
        for at in 0..bytes.len() {
            for bit in 0..8 {
                let mut changed = bytes.to_vec();
                changed[at] ^= 1 << bit;
                if let Ok(decoded) = catalog::decode_compound(&changed) {
                    assert_eq!(catalog::encode_compound(&decoded).unwrap(), changed);
                }
            }
        }
    }
}

#[test]
fn independent_lcix_v3_rejections_are_refused_with_the_expected_error() {
    macro_rules! vector {
        ($name:literal) => {
            include_bytes!(concat!(
                "../../../formats/corpus/catalog-index-v3-reject-",
                $name,
                ".golden"
            ))
            .as_slice()
        };
    }
    let cases: [(&str, &[u8], CatalogError); 9] = [
        (
            "zero-definitions",
            vector!("zero-definitions"),
            CatalogError::BadDefinition(SqlDefError::EmptyDefinitions),
        ),
        (
            "definition-block-missing",
            vector!("definition-block-missing"),
            CatalogError::BadDefinition(SqlDefError::EmptyDefinitions),
        ),
        (
            "version-2",
            vector!("version-2"),
            CatalogError::UnsupportedVersion(2),
        ),
        (
            "version-4",
            vector!("version-4"),
            CatalogError::UnsupportedVersion(4),
        ),
        (
            "version-1-with-definition-block",
            vector!("version-1-with-definition-block"),
            CatalogError::BadCompound("rows"),
        ),
        (
            "definition-count-past-end",
            vector!("definition-count-past-end"),
            CatalogError::BadDefinition(SqlDefError::BadOpTag(0)),
        ),
        (
            "definition-ids-not-ascending",
            vector!("definition-ids-not-ascending"),
            CatalogError::BadDefinition(SqlDefError::NotAscending),
        ),
        (
            "definition-unknown-op-tag",
            vector!("definition-unknown-op-tag"),
            CatalogError::BadDefinition(SqlDefError::BadOpTag(2)),
        ),
        (
            "trailing-byte",
            vector!("trailing-byte"),
            CatalogError::Trailing,
        ),
    ];
    for (name, bytes, expected) in cases {
        assert_eq!(catalog::decode_compound(bytes), Err(expected), "{name}");
    }
}

#[test]
fn version_1_bytes_are_unchanged_for_a_change_without_definitions() {
    // Hand-framed: LCIX, 1, a 23-byte LCAT block, no index operation, no rows.
    let mut expected = b"LCIX\x01".to_vec();
    expected.extend_from_slice(&23u32.to_le_bytes());
    expected.extend_from_slice(b"LCAT\x01");
    expected.extend_from_slice(&1u32.to_le_bytes());
    expected.push(1);
    expected.extend_from_slice(&7u32.to_le_bytes());
    expected.extend_from_slice(&5u32.to_le_bytes());
    expected.extend_from_slice(b"items");
    expected.extend_from_slice(&0u32.to_le_bytes());
    expected.extend_from_slice(&0u32.to_le_bytes());
    let change = CompoundChange {
        tables: vec![items_create()],
        indexes: Vec::new(),
        definitions: Vec::new(),
        rows: Vec::new(),
    };
    assert_eq!(catalog::encode_compound(&change).unwrap(), expected);
    assert_eq!(catalog::decode_compound(&expected), Ok(change));
    assert_eq!(
        catalog::encode_compound(&CompoundChange {
            tables: Vec::new(),
            indexes: Vec::new(),
            definitions: Vec::new(),
            rows: Vec::new(),
        }),
        Err(CatalogError::Empty)
    );
}

// ---------------------------------------------------------------------------------------------
// power-cut sweep over table creation

#[derive(Clone)]
struct Attempt {
    creates: Vec<u32>,
    defines: Vec<u32>,
    row_table: Option<u32>,
    acked: bool,
}

fn definition_for(id: u32) -> SqlTableDef {
    if id % 2 == 1 {
        items_def()
    } else {
        small_def()
    }
}

fn step(store: &mut Store, creates: &[u32], defines: &[u32], row_table: Option<u32>) -> Attempt {
    let mut tx = store.begin();
    if let Some(table) = row_table {
        tx.put((table, b"a".to_vec()), b"x".to_vec());
    }
    let acked = store
        .commit_with_definitions(
            tx,
            creates.iter().map(|id| create(*id)).collect(),
            Vec::new(),
            defines
                .iter()
                .map(|id| define(*id, definition_for(*id)))
                .collect(),
        )
        .is_ok();
    Attempt {
        creates: creates.to_vec(),
        defines: defines.to_vec(),
        row_table,
        acked,
    }
}

fn sweep_config(small_segments: bool) -> Config {
    Config {
        segment_limit: if small_segments { 128 } else { 4_096 },
        ..Config::default()
    }
}

/// One of 20 workloads: five shapes of table creation, with and without a legacy prefix in the
/// directory, with large and tiny (rolling) segments. Returns the number of mutating storage
/// operations the creation phase performed, or the first violated expectation.
fn creation_cut(workload: u64, cut: Option<u64>) -> Result<u64, String> {
    let shape = workload % 5;
    let legacy_prefix = (workload / 5) % 2 == 1;
    let small_segments = workload / 10 == 1;
    let disk = SharedIo::new(SimIo::new(workload, FaultProfile::clean()));
    let clock = Rc::new(SimClock::new(1_000));
    let cfg = sweep_config(small_segments);
    let mut store = try_open(&disk, &clock, &cfg).map_err(|e| format!("open: {e}"))?;
    store.set_allow_format_upgrade(true);
    if legacy_prefix {
        let mut tx = store.begin();
        tx.put((5, b"p".to_vec()), b"1".to_vec());
        store.commit(tx).map_err(|e| format!("prefix: {e}"))?;
    }
    let before = disk.mutating_ops();
    if let Some(cut) = cut {
        disk.arm_power_cut(cut);
    }
    let attempts = match shape {
        0 => vec![step(&mut store, &[7], &[7], None)],
        1 => vec![step(&mut store, &[7], &[7], Some(7))],
        2 => vec![step(&mut store, &[7, 8], &[7, 8], None)],
        3 => vec![
            step(&mut store, &[7], &[7], None),
            step(&mut store, &[8], &[8], None),
        ],
        _ => vec![
            step(&mut store, &[7], &[], None),
            step(&mut store, &[8], &[8], None),
        ],
    };
    let operations = disk.mutating_ops() - before;
    let cut_observed = disk.is_powered_off();
    drop(store);
    disk.crash();
    disk.set_profile(FaultProfile::clean());
    let recovered = try_open(&disk, &clock, &cfg).map_err(|e| format!("recovery: {e}"))?;
    let seen = check_recovered(&recovered, &attempts)?;
    let segment_masks = masks(&disk);
    for pair in segment_masks.windows(2) {
        if pair[0] & !pair[1] != 0 {
            return Err(format!(
                "capability mask shrank across segments: {segment_masks:?}"
            ));
        }
    }
    if !recovered.catalog().definitions().is_empty() && !segment_masks.iter().any(|m| m & 8 != 0) {
        return Err(format!(
            "a definition survives in an unguarded log: {segment_masks:?}"
        ));
    }
    drop(recovered);
    // A second recovery, after another restart, reproduces the same catalog.
    disk.crash();
    let again = try_open(&disk, &clock, &cfg).map_err(|e| format!("second recovery: {e}"))?;
    if check_recovered(&again, &attempts)? != seen {
        return Err("the second recovery differs from the first".into());
    }
    if cut.is_some_and(|cut| cut < operations) && !cut_observed {
        return Err(format!("cut {cut:?} of {operations} was not reached"));
    }
    Ok(operations)
}

/// Returns, per attempt, whether it is present, after checking the all-or-nothing rules.
fn check_recovered(store: &Store, attempts: &[Attempt]) -> Result<Vec<bool>, String> {
    let catalog = store.catalog();
    let mut present = Vec::new();
    for (n, attempt) in attempts.iter().enumerate() {
        let tables: Vec<bool> = attempt
            .creates
            .iter()
            .map(|id| catalog.current().contains_key(id))
            .collect();
        let defined: Vec<bool> = attempt
            .defines
            .iter()
            .map(|id| catalog.definition(*id).is_some())
            .collect();
        let here = if attempt.creates.is_empty() {
            defined.first().copied().unwrap_or(false)
        } else {
            tables.first().copied().unwrap_or(false)
        };
        if tables.iter().any(|t| *t != here) && !attempt.creates.is_empty() {
            return Err(format!(
                "attempt {n}: only some tables of one change set exist"
            ));
        }
        if defined.iter().any(|d| *d != here) && !attempt.defines.is_empty() {
            // A create with a definition in the same change set: table and definition together.
            return Err(format!(
                "attempt {n}: table present={here} but definitions present={defined:?}"
            ));
        }
        for id in &attempt.defines {
            if let Some(found) = catalog.definition(*id)
                && found != &definition_for(*id)
            {
                return Err(format!(
                    "attempt {n}: table {id} recovered a different definition"
                ));
            }
        }
        if attempt.acked && !here {
            return Err(format!("attempt {n}: an acknowledged change was lost"));
        }
        if let Some(table) = attempt.row_table {
            let expected: BTreeMap<Vec<u8>, Vec<u8>> = if here {
                BTreeMap::from([(b"a".to_vec(), b"x".to_vec())])
            } else {
                BTreeMap::new()
            };
            if rows(store, table) != expected {
                return Err(format!(
                    "attempt {n}: rows are not all-or-nothing with the table"
                ));
            }
        }
        present.push(here);
    }
    if present.windows(2).any(|w| !w[0] && w[1]) {
        return Err(format!(
            "a later change survived an earlier one: {present:?}"
        ));
    }
    let known: Vec<u32> = attempts
        .iter()
        .flat_map(|a| a.creates.iter().copied())
        .collect();
    if catalog.current().keys().any(|id| !known.contains(id)) {
        return Err("an unknown table appeared".into());
    }
    Ok(present)
}

#[test]
fn every_power_cut_during_table_creation_recovers_no_table_or_table_with_definition() {
    let mut total = 0;
    for workload in 0..20 {
        let operations =
            creation_cut(workload, None).unwrap_or_else(|e| panic!("workload {workload}: {e}"));
        assert!(operations > 0, "workload {workload} did no storage work");
        for cut in 0..=operations {
            creation_cut(workload, Some(cut))
                .unwrap_or_else(|e| panic!("workload {workload} cut {cut}: {e}"));
            total += 1;
        }
    }
    assert!(total >= 100, "swept only {total} cut points");
}

#[test]
fn the_creation_sweep_fails_under_the_definition_planted_bugs() {
    for bug in [Bug::DefinitionDroppedOnReplay, Bug::GuardBitNotRequested] {
        for workload in [0, 1, 3, 12] {
            bugs::clear();
            let guard = BugGuard;
            assert!(
                creation_cut(workload, None).is_ok(),
                "clean control {workload}"
            );
            bugs::enable(bug);
            let result = creation_cut(workload, None);
            drop(guard);
            assert!(
                result.is_err(),
                "{bug:?} was not caught by workload {workload}"
            );
        }
    }
}
