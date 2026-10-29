//! A row store on top of the kernel: rows live in memory as versions, and the log is the only
//! durable copy.
//!
//! A commit is one change set. The conflict rule is checked first (first committer wins, see
//! `formats/isolation.md`), the change set goes through the kernel's commit path, and only when the
//! kernel acknowledges it durable are its writes applied to the in-memory versions at the commit LSN,
//! in one call, so every reader sees all of the change set or none of it. Recovery replays the log's
//! row change sets through the same apply, so a restart reproduces exactly the acknowledged state.

use crate::catalog::{self, Catalog, CatalogError, CatalogOp};
use crate::index::{IndexDef, Indexes};
use crate::mvcc::{MvccError, Snapshot, Table, Tx};
use crate::rowops::{self, RowKey, RowOpsError};
use lattice_abi::{Classify, ErrorClass};
use lattice_core::bugs::{self, Bug};
use lattice_core::changeset::ChangeSet;
use lattice_core::engine_checkpoint::{CheckpointRecovery, StateRead, StateWrite};
use lattice_core::record::{Record, RecordType};
use lattice_core::{Config, Kernel, LogError, Receipt, RecoveryReport, TxnError};
use lattice_seams::{Clock, Io, Rng};
use std::collections::BTreeSet;
use std::fmt;
use std::ops::{Bound, RangeBounds};

/// A transaction on the store.
pub type RowTx = Tx<RowKey, Vec<u8>>;

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq)]
pub enum RowError {
    /// Another change set wrote one of the same rows after this transaction's snapshot. Safe to
    /// retry as a new transaction (SQLSTATE 40001).
    Conflict {
        /// The LSN of the newer write.
        newer_lsn: u64,
    },
    /// The kernel refused the change set; nothing was written.
    Rejected(TxnError),
    /// The commit failed in a way that leaves it unknown whether it is durable. The store is stale:
    /// reopen it, and the log decides.
    InDoubt(TxnError),
    /// An earlier ambiguous failure left this store behind the log; reopen it.
    Stale,
    /// The log holds a row change set this code cannot read.
    Corrupt(String),
    /// The index definitions cannot identify their entries unambiguously.
    InvalidIndexes(String),
    /// Catalog rules rejected the operation before writing anything.
    InvalidCatalog(CatalogError),
    /// A raw write violates a catalog-owned typed table schema.
    InvalidTyped(String),
    /// The writes do not fit the payload format.
    Encode(RowOpsError),
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowError::Conflict { newer_lsn } => {
                write!(f, "write conflict: a row was changed at lsn {newer_lsn}")
            }
            RowError::Rejected(e) => write!(f, "rejected: {e}"),
            RowError::InDoubt(e) => write!(f, "commit outcome unknown: {e}"),
            RowError::Stale => write!(f, "store is stale after an ambiguous failure; reopen it"),
            RowError::Corrupt(why) => write!(f, "row data corrupt: {why}"),
            RowError::InvalidCatalog(why) => write!(f, "invalid catalog operation: {why}"),
            RowError::InvalidIndexes(why) => write!(f, "invalid index definitions: {why}"),
            RowError::InvalidTyped(why) => write!(f, "invalid typed row: {why}"),
            RowError::Encode(e) => write!(f, "cannot encode writes: {e:?}"),
        }
    }
}

impl Classify for RowError {
    fn class(&self) -> ErrorClass {
        match self {
            RowError::Conflict { .. } | RowError::Stale => ErrorClass::Retryable,
            RowError::Rejected(e) | RowError::InDoubt(e) => e.class(),
            RowError::Corrupt(_) => ErrorClass::Fatal,
            RowError::Encode(_)
            | RowError::InvalidIndexes(_)
            | RowError::InvalidCatalog(_)
            | RowError::InvalidTyped(_) => ErrorClass::Rejected,
        }
    }
}

// Catalog-only validation cannot see uncataloged physical row ids. The shared row/catalog
// boundary must reject assigning a typed layout to any id with older untyped row history.
fn check_schema_history(
    catalog: &Catalog,
    row_tables: &BTreeSet<u32>,
    ops: &[CatalogOp],
) -> Result<(), CatalogError> {
    for op in ops {
        if let CatalogOp::DefineSchema { tables, .. } = op {
            for table in tables.keys() {
                if row_tables.contains(table)
                    && catalog.layout_at(*table, catalog.applied_lsn()).is_none()
                {
                    return Err(CatalogError::IdUsed(*table));
                }
            }
        }
    }
    Ok(())
}

fn checkpoint_bad(why: &str) -> LogError {
    LogError::Corrupt(format!("row checkpoint: {why}"))
}
fn encode_rows(
    table: &Table<RowKey, Vec<u8>>,
    writer: &mut dyn StateWrite,
) -> Result<(), LogError> {
    writer.u64(table.applied_lsn())?;
    writer.u64(table.history().count() as u64)?;
    for ((id, key), lsn, cs, value) in table.history() {
        writer.u64(u64::from(*id))?;
        writer.bytes(key)?;
        writer.u64(lsn)?;
        writer.raw(&cs.to_bytes())?;
        writer.u64(u64::from(value.is_some()))?;
        if let Some(value) = value {
            writer.bytes(value)?;
        }
    }
    Ok(())
}
fn restore_rows(
    reader: &mut dyn StateRead,
    through: u64,
    seen: &std::collections::BTreeSet<lattice_core::ChangeSetId>,
) -> Result<Table<RowKey, Vec<u8>>, LogError> {
    let applied = reader.u64()?;
    if applied > through {
        return Err(checkpoint_bad("applied LSN beyond source"));
    }
    let count = reader.count(48)?;
    let mut table = Table::new();
    let mut previous: Option<(RowKey, u64)> = None;
    for _ in 0..count {
        let id = u32::try_from(reader.u64()?).map_err(|_| checkpoint_bad("table id"))?;
        let key = (id, reader.bytes()?);
        let lsn = reader.u64()?;
        let mut bytes = [0; 16];
        reader.raw(&mut bytes)?;
        let cs = lattice_core::ChangeSetId::from_bytes(bytes);
        if lsn == 0
            || lsn > applied
            || !seen.contains(&cs)
            || previous
                .as_ref()
                .is_some_and(|p| (&key, lsn) <= (&p.0, p.1))
        {
            return Err(checkpoint_bad("version order, LSN or change set"));
        }
        let value = match reader.u64()? {
            0 => None,
            1 => Some(reader.bytes()?),
            _ => return Err(checkpoint_bad("value tag")),
        };
        table
            .restore_version(key.clone(), lsn, cs, value)
            .map_err(|_| checkpoint_bad("version order"))?;
        previous = Some((key, lsn));
    }
    if table.applied_lsn() > applied {
        return Err(checkpoint_bad("applied LSN"));
    }
    table.restore_applied(applied);
    Ok(table)
}

/// Rows over the kernel.
pub struct RowStore<I: Io, C: Clock, R: Rng> {
    kernel: Kernel<I, C, R>,
    table: Table<RowKey, Vec<u8>>,
    indexes: Indexes,
    catalog: Catalog,
    /// Physical ids observed in any row write, including deletions and historical versions.
    row_tables: BTreeSet<u32>,
    stale: bool,
    recovery: (RecoveryReport, CheckpointRecovery),
}

impl<I: Io, C: Clock, R: Rng> RowStore<I, C, R> {
    /// Opens the log and rebuilds the rows from every row change set in it, streaming, so recovery
    /// memory is the row state itself and one log segment.
    pub fn open(io: I, clock: C, rng: R, config: &Config) -> Result<Self, RowError> {
        Self::open_with_indexes(io, clock, rng, config, Vec::new())
    }

    /// Opens the store with indexes derived from the row history. Index definitions must be
    /// deterministic and use distinct ids, and must be supplied again on every reopen.
    pub fn open_with_indexes(
        io: I,
        clock: C,
        rng: R,
        config: &Config,
        index_defs: Vec<IndexDef>,
    ) -> Result<Self, RowError> {
        let mut ids = std::collections::BTreeSet::new();
        if index_defs.iter().any(|def| !ids.insert(def.id)) {
            return Err(RowError::InvalidIndexes("duplicate index id".into()));
        }
        let (kernel, (table, indexes, catalog, row_tables), report, checkpoint) =
            Kernel::open_checkpointed(
                io,
                clock,
                rng,
                config,
                b"typed-row-catalog-history-v1",
                || {
                    (
                        Table::new(),
                        Indexes::new(index_defs.clone()),
                        Catalog::new(),
                        BTreeSet::new(),
                    )
                },
                |reader, through, seen| {
                    let catalog = Catalog::restore_checkpoint(reader, through)?;
                    let mut row_tables = BTreeSet::new();
                    let mut previous = None;
                    for _ in 0..reader.count(8)? {
                        let id = u32::try_from(reader.u64()?)
                            .map_err(|_| checkpoint_bad("reserved raw table"))?;
                        if previous.is_some_and(|p| p >= id) {
                            return Err(checkpoint_bad("reserved table order"));
                        }
                        row_tables.insert(id);
                        previous = Some(id);
                    }
                    let table = restore_rows(reader, through, seen)?;
                    for (key, lsn, _, value) in table.history() {
                        if catalog
                            .first_typed_lsn(key.0)
                            .is_some_and(|first| lsn <= first)
                        {
                            return Err(checkpoint_bad(
                                "typed layout adopts older raw row history",
                            ));
                        }
                        if !row_tables.contains(&key.0) {
                            return Err(checkpoint_bad("unreserved historical row table"));
                        }
                        crate::typed_store::validate_value_at(
                            &catalog,
                            key.0,
                            value.map(Vec::as_slice),
                            lsn,
                        )
                        .map_err(|e| checkpoint_bad(&e))?;
                    }
                    let mut indexes = Indexes::new(index_defs.clone());
                    indexes
                        .restore_history(&table)
                        .map_err(|e| checkpoint_bad(&format!("index restore: {e:?}")))?;
                    Ok((table, indexes, catalog, row_tables))
                },
                |r: Record, (table, indexes, catalog, row_tables)| {
                    if RecordType::from_u8(r.rtype) != Some(RecordType::ChangeSet) {
                        return Ok(());
                    }
                    let cs = ChangeSet::decode(&r.payload)
                        .map_err(|e| checkpoint_bad(&format!("change set: {e}")))?;
                    if catalog::is_catalog_payload(&cs.payload) {
                        let ops = catalog::decode(&cs.payload)
                            .map_err(|e| checkpoint_bad(&format!("catalog: {e}")))?;
                        check_schema_history(catalog, row_tables, &ops)
                            .map_err(|e| checkpoint_bad(&format!("catalog history: {e}")))?;
                        catalog
                            .apply(r.lsn, &ops)
                            .map_err(|e| checkpoint_bad(&format!("catalog apply: {e}")))?;
                        return Ok(());
                    }
                    if !rowops::is_row_payload(&cs.payload) {
                        return Ok(());
                    }
                    let mut writes = rowops::decode(&cs.payload)
                        .map_err(|e| checkpoint_bad(&format!("row writes: {e:?}")))?;
                    crate::typed_store::validate_writes(catalog, &writes)
                        .map_err(|e| checkpoint_bad(&format!("typed rows: {e}")))?;
                    row_tables.extend(writes.iter().map(|((table, _), _)| *table));
                    if bugs::active(Bug::RowRecoveryDropsDeletes) {
                        writes.retain(|(_, value)| value.is_some());
                    }
                    let changes = indexes.changes_for(table, &writes);
                    table
                        .apply(r.lsn, cs.id, writes)
                        .and_then(|()| indexes.apply(r.lsn, cs.id, changes))
                        .map_err(|e| checkpoint_bad(&format!("row apply: {e:?}")))
                },
            )
            .map_err(|error| match error {
                TxnError::Log(LogError::Corrupt(why)) => RowError::Corrupt(why),
                other => RowError::Rejected(other),
            })?;
        Ok(Self {
            kernel,
            table,
            indexes,
            catalog,
            row_tables,
            stale: false,
            recovery: (report, checkpoint),
        })
    }

    /// Publishes catalog schemas, physical-id reservations, all retained rows and kernel state.
    pub fn checkpoint(&mut self) -> Result<u64, RowError> {
        if self.stale {
            return Err(RowError::Stale);
        }
        match self
            .kernel
            .checkpoint_engine(b"typed-row-catalog-history-v1", |writer| {
                self.catalog.encode_checkpoint(writer)?;
                writer.u64(self.row_tables.len() as u64)?;
                for id in &self.row_tables {
                    writer.u64(u64::from(*id))?;
                }
                encode_rows(&self.table, writer)
            }) {
            Ok(lsn) => Ok(lsn),
            Err(error) => {
                self.stale = true;
                Err(RowError::InDoubt(error))
            }
        }
    }

    /// Evidence of checkpoint state and suffix replay for this opening.
    pub fn recovery_report(&self) -> &(RecoveryReport, CheckpointRecovery) {
        &self.recovery
    }

    /// Starts a transaction on the newest snapshot.
    pub fn begin(&self) -> RowTx {
        Tx::begin(&self.table)
    }

    /// The newest snapshot: everything committed so far.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            as_of: self.kernel.durable_lsn(),
        }
    }

    /// The versioned catalog sharing this store's WAL and writer.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Whether this physical table id has appeared in any durable row write. Deletions do not
    /// release ids: tombstones and all historical versions reserve the id even without a catalog.
    /// Recovery rebuilds this set from the WAL independently of live row visibility.
    pub fn has_table_history(&self, table: u32) -> bool {
        self.row_tables.contains(&table)
    }

    /// Commits catalog changes through the same writer as row changes.
    pub fn commit_catalog(&mut self, ops: Vec<CatalogOp>) -> Result<u64, RowError> {
        if self.stale {
            return Err(RowError::Stale);
        }
        self.catalog.check(&ops).map_err(RowError::InvalidCatalog)?;
        check_schema_history(&self.catalog, &self.row_tables, &ops)
            .map_err(RowError::InvalidCatalog)?;
        let payload = catalog::encode(&ops).map_err(RowError::InvalidCatalog)?;
        let id = self.kernel.new_id();
        match self.kernel.commit(ChangeSet::new(id, payload)) {
            Ok(Receipt::Recorded { lsn, .. }) => {
                self.catalog
                    .apply(lsn, &ops)
                    .map_err(|e| RowError::Corrupt(format!("catalog apply after commit: {e}")))?;
                Ok(lsn)
            }
            Ok(Receipt::Duplicate) => {
                self.stale = true;
                Err(RowError::Corrupt("fresh catalog id was a duplicate".into()))
            }
            Err(e) if e.class() == ErrorClass::Rejected => Err(RowError::Rejected(e)),
            Err(e) => {
                self.stale = true;
                Err(RowError::InDoubt(e))
            }
        }
    }

    /// Reads a row inside a transaction: its own writes first, then its snapshot.
    pub fn get<'a>(&'a self, tx: &'a RowTx, table: u32, key: &[u8]) -> Option<&'a Vec<u8>> {
        tx.get(&self.table, &(table, key.to_vec()))
    }

    /// Reads a row as of a snapshot.
    pub fn get_at(&self, snap: Snapshot, table: u32, key: &[u8]) -> Option<&Vec<u8>> {
        self.table.get(snap, &(table, key.to_vec()))
    }

    /// Every live row of `table` with a key in `range` as of `snap`, in key order.
    pub fn scan_at<B: RangeBounds<Vec<u8>>>(
        &self,
        snap: Snapshot,
        table: u32,
        range: B,
    ) -> Vec<(&[u8], &Vec<u8>)> {
        let lo = match range.start_bound() {
            Bound::Included(k) => Bound::Included((table, k.clone())),
            Bound::Excluded(k) => Bound::Excluded((table, k.clone())),
            Bound::Unbounded => Bound::Included((table, Vec::new())),
        };
        let hi = match range.end_bound() {
            Bound::Included(k) => Bound::Included((table, k.clone())),
            Bound::Excluded(k) if bugs::active(Bug::RowScanUpperBoundIncluded) => {
                Bound::Included((table, k.clone()))
            }
            Bound::Excluded(k) => Bound::Excluded((table, k.clone())),
            Bound::Unbounded => match table.checked_add(1) {
                Some(next) => Bound::Excluded((next, Vec::new())),
                None => Bound::Unbounded,
            },
        };
        self.table
            .range(snap, (lo, hi))
            .into_iter()
            .map(|((_, k), v)| (k.as_slice(), v))
            .collect()
    }

    /// Primary keys whose extracted index key equals `key` at `snap`, in primary-key order.
    pub fn lookup_at(&self, snap: Snapshot, index: u32, key: &[u8]) -> Vec<Vec<u8>> {
        self.indexes.lookup(snap, index, key)
    }

    /// Rebuilds expected index entries from the rows at `snap` and reports any difference (I9).
    pub fn check_indexes_at(&self, snap: Snapshot) -> Result<(), String> {
        let live = self.indexes.live_entries(snap);
        let expected = self.indexes.expected_entries(&self.table, snap);
        if live == expected {
            Ok(())
        } else {
            Err(format!(
                "indexes differ from the rows: {} stale or extra, {} missing",
                live.iter()
                    .filter(|(key, value)| expected.get(*key) != Some(*value))
                    .count(),
                expected
                    .iter()
                    .filter(|(key, value)| live.get(*key) != Some(*value))
                    .count()
            ))
        }
    }

    /// Checks index consistency at the current snapshot (I9).
    pub fn check_indexes(&self) -> Result<(), String> {
        self.check_indexes_at(self.snapshot())
    }

    /// The newest row change set applied. [`Self::snapshot`] also includes catalog-only commits.
    pub fn applied_lsn(&self) -> u64 {
        self.table.applied_lsn()
    }

    /// Commits a transaction as one change set and returns its commit LSN (the snapshot LSN for a
    /// transaction that wrote nothing).
    ///
    /// Catalog-owned typed tables enforce their current schema here too. Raw tables remain byte
    /// oriented. Use `TypedStore` to construct valid typed envelopes and retain schema fencing.
    pub fn commit(&mut self, tx: RowTx) -> Result<u64, RowError> {
        if self.stale {
            return Err(RowError::Stale);
        }
        if tx.is_read_only() {
            return Ok(tx.snapshot().as_of);
        }
        if !bugs::active(Bug::RowConflictCheckSkipped) {
            tx.check(&self.table).map_err(|e| match e {
                MvccError::Conflict { newer_lsn } => RowError::Conflict { newer_lsn },
                MvccError::OutOfOrder { .. } => RowError::Stale,
            })?;
        }
        let writes = tx.into_writes();
        crate::typed_store::validate_writes(&self.catalog, &writes)
            .map_err(RowError::InvalidTyped)?;
        let payload = rowops::encode(&writes).map_err(RowError::Encode)?;
        let id = self.kernel.new_id();
        match self.kernel.commit(ChangeSet::new(id, payload)) {
            Ok(Receipt::Recorded { lsn, .. }) => {
                let writes: Vec<_> = if bugs::active(Bug::RowPartialApply) {
                    writes.into_iter().take(1).collect()
                } else {
                    writes
                };
                self.row_tables
                    .extend(writes.iter().map(|((table, _), _)| *table));
                let changes = self.indexes.changes_for(&self.table, &writes);
                self.table
                    .apply(lsn, id, writes)
                    .and_then(|()| self.indexes.apply(lsn, id, changes))
                    .map_err(|e| RowError::Corrupt(format!("apply after commit: {e:?}")))?;
                Ok(lsn)
            }
            Ok(Receipt::Duplicate) => {
                self.stale = true;
                Err(RowError::Corrupt(
                    "a fresh change set id was a duplicate".into(),
                ))
            }
            Err(e) if e.class() == ErrorClass::Rejected => Err(RowError::Rejected(e)),
            Err(e) => {
                // It may or may not have reached the disk; only the log knows.
                self.stale = true;
                Err(RowError::InDoubt(e))
            }
        }
    }
}

#[cfg(test)]
mod checkpoint_golden {
    #![allow(clippy::unwrap_used)]
    use super::*;
    struct Bytes<'a>(&'a [u8]);
    impl StateRead for Bytes<'_> {
        fn remaining(&self) -> u64 {
            self.0.len() as u64
        }
        fn raw(&mut self, out: &mut [u8]) -> Result<(), LogError> {
            let (a, b) = self
                .0
                .split_at_checked(out.len())
                .ok_or_else(|| checkpoint_bad("short golden"))?;
            out.copy_from_slice(a);
            self.0 = b;
            Ok(())
        }
    }
    #[test]
    fn independently_encoded_row_history_reencodes_exactly() {
        let expected =
            include_bytes!("../../../formats/corpus/checkpoint-v1/row/engine-state.golden");
        let seen = (1u128..=3)
            .map(|n| lattice_core::ChangeSetId::from_bytes(n.to_be_bytes()))
            .collect();
        let mut reader = Bytes(expected);
        let table = restore_rows(&mut reader, 4, &seen).unwrap();
        assert_eq!(reader.remaining(), 0);
        let mut actual = Vec::new();
        encode_rows(&table, &mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}
