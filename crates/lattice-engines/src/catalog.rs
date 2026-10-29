//! The catalog: which tables exist, kept in the log like everything else.
//!
//! A catalog change is a change set whose payload starts with the magic `LCAT`. Applying it is one step
//! at one LSN, and the catalog keeps every version, so it can be read as of any earlier LSN. Table ids
//! are never reused, so an old id in a row change set always means the table it meant then.
//!
//! Layout (little endian): magic `LCAT`, a version byte, a `u32` count, then per operation a kind byte
//! (1 create, 2 rename, 3 drop), a `u32` table id and, for create and rename, a `u32` name length and
//! the UTF-8 name. Decoding is strict, like the row payloads.

use lattice_abi::{Classify, ErrorClass};
use lattice_core::changeset::ChangeSet;
use lattice_core::engine_checkpoint::{CheckpointRecovery, StateRead, StateWrite};
use lattice_core::record::{Record, RecordType};
use lattice_core::{Config, Kernel, Receipt, TxnError};
use lattice_core::{LogError, RecoveryReport};
use lattice_seams::{Clock, Io, Rng};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The first bytes of every catalog payload.
pub const MAGIC: [u8; 4] = *b"LCAT";
const VERSION: u8 = 1;
/// Longest table name in bytes.
pub const MAX_NAME: usize = 255;

/// One catalog change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogOp {
    /// Add a table.
    Create {
        /// The new table's id; never used before.
        table: u32,
        /// Its name; unique among live tables.
        name: String,
    },
    /// Change a table's name.
    Rename {
        /// The table.
        table: u32,
        /// The new name; unique among live tables.
        name: String,
    },
    /// Remove a table. Its id is never reused.
    Drop {
        /// The table.
        table: u32,
    },
}

/// Why a catalog change or payload was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The id has been used before.
    IdUsed(u32),
    /// A live table already has this name.
    NameTaken(String),
    /// No live table has this id.
    NoSuchTable(u32),
    /// The name is empty, longer than [`MAX_NAME`], or not valid UTF-8.
    BadName,
    /// Change sets must be applied in increasing LSN order.
    OutOfOrder,
    /// There are no operations.
    Empty,
    /// The bytes end inside a field.
    Truncated,
    /// A kind byte other than 1, 2 or 3.
    BadKind(u8),
    /// A format version this code cannot read.
    UnsupportedVersion(u8),
    /// Bytes remain after the last operation.
    Trailing,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Whether `payload` is a catalog payload.
pub fn is_catalog_payload(payload: &[u8]) -> bool {
    payload.get(..4) == Some(MAGIC.as_slice())
}

/// Encodes operations.
pub fn encode(ops: &[CatalogOp]) -> Result<Vec<u8>, CatalogError> {
    if ops.is_empty() {
        return Err(CatalogError::Empty);
    }
    let mut out = MAGIC.to_vec();
    out.push(VERSION);
    out.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    for op in ops {
        let (kind, table, name) = match op {
            CatalogOp::Create { table, name } => (1u8, *table, Some(name)),
            CatalogOp::Rename { table, name } => (2, *table, Some(name)),
            CatalogOp::Drop { table } => (3, *table, None),
        };
        out.push(kind);
        out.extend_from_slice(&table.to_le_bytes());
        if let Some(name) = name {
            if name.is_empty() || name.len() > MAX_NAME {
                return Err(CatalogError::BadName);
            }
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
    }
    Ok(out)
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CatalogError> {
        if self.rest.len() < n {
            return Err(CatalogError::Truncated);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, CatalogError> {
        self.take(1)?
            .first()
            .copied()
            .ok_or(CatalogError::Truncated)
    }

    fn u32(&mut self) -> Result<u32, CatalogError> {
        let b: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| CatalogError::Truncated)?;
        Ok(u32::from_le_bytes(b))
    }

    fn name(&mut self) -> Result<String, CatalogError> {
        let len = self.u32()? as usize;
        if len == 0 || len > MAX_NAME {
            return Err(CatalogError::BadName);
        }
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| CatalogError::BadName)
    }
}

/// Decodes a catalog payload; total, with no allocation from an unchecked length.
pub fn decode(payload: &[u8]) -> Result<Vec<CatalogOp>, CatalogError> {
    let mut r = Reader { rest: payload };
    if r.take(4)? != MAGIC.as_slice() {
        return Err(CatalogError::Truncated);
    }
    match r.u8()? {
        VERSION => {}
        v => return Err(CatalogError::UnsupportedVersion(v)),
    }
    let count = r.u32()?;
    if count == 0 {
        return Err(CatalogError::Empty);
    }
    let mut ops = Vec::new();
    for _ in 0..count {
        let kind = r.u8()?;
        let table = r.u32()?;
        ops.push(match kind {
            1 => CatalogOp::Create {
                table,
                name: r.name()?,
            },
            2 => CatalogOp::Rename {
                table,
                name: r.name()?,
            },
            3 => CatalogOp::Drop { table },
            k => return Err(CatalogError::BadKind(k)),
        });
    }
    if !r.rest.is_empty() {
        return Err(CatalogError::Trailing);
    }
    Ok(ops)
}

/// The tables that exist at one version: id to name.
pub type Tables = BTreeMap<u32, String>;

fn catalog_checkpoint_bad(why: &str) -> LogError {
    LogError::Corrupt(format!("catalog checkpoint: {why}"))
}

/// Every version of the catalog.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// (LSN of the change, tables right after it), in increasing LSN order.
    versions: Vec<(u64, Tables)>,
    /// Every id ever created, live or dropped.
    used: BTreeSet<u32>,
}

impl Catalog {
    fn encode_checkpoint(&self, writer: &mut dyn StateWrite) -> Result<(), LogError> {
        writer.u64(self.used.len() as u64)?;
        for id in &self.used {
            writer.u64(u64::from(*id))?;
        }
        writer.u64(self.versions.len() as u64)?;
        for (lsn, tables) in &self.versions {
            writer.u64(*lsn)?;
            writer.u64(tables.len() as u64)?;
            for (id, name) in tables {
                writer.u64(u64::from(*id))?;
                writer.bytes(name.as_bytes())?;
            }
        }
        Ok(())
    }

    fn restore_checkpoint(reader: &mut dyn StateRead, through: u64) -> Result<Self, LogError> {
        let mut used = BTreeSet::new();
        let mut previous = None;
        for _ in 0..reader.count(8)? {
            let id = u32::try_from(reader.u64()?).map_err(|_| catalog_checkpoint_bad("used id"))?;
            if previous.is_some_and(|p| p >= id) {
                return Err(catalog_checkpoint_bad("used id order"));
            }
            used.insert(id);
            previous = Some(id);
        }
        let count = reader.count(16)?;
        if count > through {
            return Err(catalog_checkpoint_bad("version count"));
        }
        let mut versions = Vec::new();
        let mut last_lsn = 0;
        let mut appeared = BTreeSet::new();
        let mut dropped = BTreeSet::new();
        let mut prior = BTreeSet::new();
        for _ in 0..count {
            let lsn = reader.u64()?;
            if lsn <= last_lsn || lsn > through {
                return Err(catalog_checkpoint_bad("version LSN"));
            }
            let mut tables = BTreeMap::new();
            let mut names = BTreeSet::new();
            let mut prev = None;
            for _ in 0..reader.count(17)? {
                let id =
                    u32::try_from(reader.u64()?).map_err(|_| catalog_checkpoint_bad("table id"))?;
                let name = String::from_utf8(reader.bytes()?)
                    .map_err(|_| catalog_checkpoint_bad("name encoding"))?;
                if name.is_empty()
                    || name.len() > MAX_NAME
                    || !used.contains(&id)
                    || dropped.contains(&id)
                    || prev.is_some_and(|p| p >= id)
                    || !names.insert(name.clone())
                {
                    return Err(catalog_checkpoint_bad("table/name/order"));
                }
                tables.insert(id, name);
                prev = Some(id);
                appeared.insert(id);
            }
            let current: BTreeSet<_> = tables.keys().copied().collect();
            dropped.extend(prior.difference(&current).copied());
            prior = current;
            versions.push((lsn, tables));
            last_lsn = lsn;
        }
        // IDs created and dropped atomically may occur only in `used`, never in a visible version.
        if !appeared.is_subset(&used) {
            return Err(catalog_checkpoint_bad("used ids"));
        }
        Ok(Self { versions, used })
    }

    /// An empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// The tables right now.
    pub fn current(&self) -> Tables {
        self.versions
            .last()
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    }

    /// The tables as of `lsn`: what the catalog said after the last change at or before it.
    pub fn as_of(&self, lsn: u64) -> Tables {
        let end = self.versions.partition_point(|(l, _)| *l <= lsn);
        end.checked_sub(1)
            .and_then(|i| self.versions.get(i))
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    }

    /// The id of the live table called `name` as of `lsn`.
    pub fn id_of(&self, lsn: u64, name: &str) -> Option<u32> {
        self.as_of(lsn)
            .into_iter()
            .find_map(|(id, n)| (n == name).then_some(id))
    }

    /// The LSN of the newest change, or 0.
    pub fn applied_lsn(&self) -> u64 {
        self.versions.last().map_or(0, |(l, _)| *l)
    }

    /// Checks the operations against the current catalog without changing it.
    pub fn check(&self, ops: &[CatalogOp]) -> Result<Tables, CatalogError> {
        let mut tables = self.current();
        let mut used = self.used.clone();
        for op in ops {
            match op {
                CatalogOp::Create { table, name } => {
                    if name.is_empty() || name.len() > MAX_NAME {
                        return Err(CatalogError::BadName);
                    }
                    if !used.insert(*table) {
                        return Err(CatalogError::IdUsed(*table));
                    }
                    if tables.values().any(|n| n == name) {
                        return Err(CatalogError::NameTaken(name.clone()));
                    }
                    tables.insert(*table, name.clone());
                }
                CatalogOp::Rename { table, name } => {
                    if name.is_empty() || name.len() > MAX_NAME {
                        return Err(CatalogError::BadName);
                    }
                    if !tables.contains_key(table) {
                        return Err(CatalogError::NoSuchTable(*table));
                    }
                    if tables.iter().any(|(id, n)| n == name && id != table) {
                        return Err(CatalogError::NameTaken(name.clone()));
                    }
                    tables.insert(*table, name.clone());
                }
                CatalogOp::Drop { table } => {
                    if tables.remove(table).is_none() {
                        return Err(CatalogError::NoSuchTable(*table));
                    }
                }
            }
        }
        Ok(tables)
    }

    /// Applies one change set's operations atomically at `lsn`; on error nothing changes.
    pub fn apply(&mut self, lsn: u64, ops: &[CatalogOp]) -> Result<(), CatalogError> {
        if lsn <= self.applied_lsn() {
            return Err(CatalogError::OutOfOrder);
        }
        let tables = self.check(ops)?;
        for op in ops {
            if let CatalogOp::Create { table, .. } = op {
                self.used.insert(*table);
            }
        }
        self.versions.push((lsn, tables));
        Ok(())
    }
}

/// The catalog on top of the kernel: every change is a change set in the shared log.
pub struct CatalogStore<I: Io, C: Clock, R: Rng> {
    kernel: Kernel<I, C, R>,
    catalog: Catalog,
    stale: bool,
    recovery: (RecoveryReport, CheckpointRecovery),
}

/// Why a catalog store operation failed.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreError {
    /// The change breaks a catalog rule; nothing was written.
    Invalid(CatalogError),
    /// The kernel refused the change set; nothing was written.
    Rejected(TxnError),
    /// It is unknown whether the change reached the disk; reopen and the log decides.
    InDoubt(TxnError),
    /// An earlier ambiguous failure left this store behind the log; reopen it.
    Stale,
    /// The log holds a catalog change this code cannot apply.
    Corrupt(String),
}

impl Classify for StoreError {
    fn class(&self) -> ErrorClass {
        match self {
            StoreError::Invalid(_) => ErrorClass::Rejected,
            StoreError::Rejected(e) | StoreError::InDoubt(e) => e.class(),
            StoreError::Stale => ErrorClass::Retryable,
            StoreError::Corrupt(_) => ErrorClass::Fatal,
        }
    }
}

impl<I: Io, C: Clock, R: Rng> CatalogStore<I, C, R> {
    /// Opens the log and rebuilds the catalog from its catalog change sets, skipping other engines'.
    pub fn open(io: I, clock: C, rng: R, config: &Config) -> Result<Self, StoreError> {
        let (kernel, catalog, report, checkpoint) = Kernel::open_checkpointed(
            io,
            clock,
            rng,
            config,
            b"legacy-catalog-history-v1",
            Catalog::new,
            |reader, through, _| Catalog::restore_checkpoint(reader, through),
            |r: Record, catalog| {
                if RecordType::from_u8(r.rtype) != Some(RecordType::ChangeSet) {
                    return Ok(());
                }
                let cs = ChangeSet::decode(&r.payload)
                    .map_err(|e| catalog_checkpoint_bad(&format!("change set: {e}")))?;
                if !is_catalog_payload(&cs.payload) {
                    return Ok(());
                }
                let ops = decode(&cs.payload)
                    .map_err(|e| catalog_checkpoint_bad(&format!("catalog: {e}")))?;
                catalog
                    .apply(r.lsn, &ops)
                    .map_err(|e| catalog_checkpoint_bad(&format!("catalog lsn {}: {e}", r.lsn)))
            },
        )
        .map_err(StoreError::Rejected)?;
        Ok(Self {
            kernel,
            catalog,
            stale: false,
            recovery: (report, checkpoint),
        })
    }

    /// Publishes all catalog versions and every used id alongside kernel state.
    pub fn checkpoint(&mut self) -> Result<u64, StoreError> {
        if self.stale {
            return Err(StoreError::Stale);
        }
        match self
            .kernel
            .checkpoint_engine(b"legacy-catalog-history-v1", |writer| {
                self.catalog.encode_checkpoint(writer)
            }) {
            Ok(lsn) => Ok(lsn),
            Err(e) => {
                self.stale = true;
                Err(StoreError::InDoubt(e))
            }
        }
    }

    /// Evidence of prefix skipping and suffix replay for this opening.
    pub fn recovery_report(&self) -> &(RecoveryReport, CheckpointRecovery) {
        &self.recovery
    }

    /// The catalog as it stands.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Commits catalog operations as one change set and returns its LSN.
    pub fn commit(&mut self, ops: Vec<CatalogOp>) -> Result<u64, StoreError> {
        if self.stale {
            return Err(StoreError::Stale);
        }
        self.catalog.check(&ops).map_err(StoreError::Invalid)?;
        let payload = encode(&ops).map_err(StoreError::Invalid)?;
        let id = self.kernel.new_id();
        match self.kernel.commit(ChangeSet::new(id, payload)) {
            Ok(Receipt::Recorded { lsn, .. }) => {
                self.catalog
                    .apply(lsn, &ops)
                    .map_err(|e| StoreError::Corrupt(format!("apply after commit: {e}")))?;
                Ok(lsn)
            }
            Ok(Receipt::Duplicate) => {
                self.stale = true;
                Err(StoreError::Corrupt(
                    "a fresh change set id was a duplicate".into(),
                ))
            }
            Err(e) if e.class() == ErrorClass::Rejected => Err(StoreError::Rejected(e)),
            Err(e) => {
                self.stale = true;
                Err(StoreError::InDoubt(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn independently_encoded_catalog_history_reencodes_exactly() {
        struct Bytes<'a>(&'a [u8]);
        impl StateRead for Bytes<'_> {
            fn remaining(&self) -> u64 {
                self.0.len() as u64
            }
            fn raw(&mut self, out: &mut [u8]) -> Result<(), LogError> {
                let (a, b) = self
                    .0
                    .split_at_checked(out.len())
                    .ok_or_else(|| catalog_checkpoint_bad("short golden"))?;
                out.copy_from_slice(a);
                self.0 = b;
                Ok(())
            }
        }
        let expected =
            include_bytes!("../../../formats/corpus/checkpoint-v1/catalog/engine-state.golden");
        let mut reader = Bytes(expected);
        let catalog = Catalog::restore_checkpoint(&mut reader, 4).unwrap();
        assert_eq!(reader.remaining(), 0);
        let mut actual = Vec::new();
        catalog.encode_checkpoint(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    fn create(table: u32, name: &str) -> CatalogOp {
        CatalogOp::Create {
            table,
            name: name.into(),
        }
    }

    fn op() -> impl Strategy<Value = CatalogOp> {
        let name = "[a-z]{1,12}";
        prop_oneof![
            (any::<u32>(), name).prop_map(|(table, name)| CatalogOp::Create { table, name }),
            (any::<u32>(), name).prop_map(|(table, name)| CatalogOp::Rename { table, name }),
            any::<u32>().prop_map(|table| CatalogOp::Drop { table }),
        ]
    }

    proptest! {
        #[test]
        fn encode_then_decode_is_identity(ops in proptest::collection::vec(op(), 1..8)) {
            prop_assert_eq!(decode(&encode(&ops).unwrap()), Ok(ops));
        }

        #[test]
        fn arbitrary_bytes_never_panic_and_a_success_re_encodes_identically(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
            if let Ok(ops) = decode(&bytes) {
                prop_assert_eq!(encode(&ops).unwrap(), bytes);
            }
        }

        #[test]
        fn every_truncation_is_an_error(ops in proptest::collection::vec(op(), 1..5)) {
            let bytes = encode(&ops).unwrap();
            for cut in 0..bytes.len() {
                prop_assert!(decode(&bytes[..cut]).is_err(), "cut at {}", cut);
            }
        }
    }

    #[test]
    fn the_catalog_can_be_read_as_of_any_earlier_lsn() {
        let mut c = Catalog::new();
        c.apply(3, &[create(1, "issues"), create(2, "pages")])
            .unwrap();
        c.apply(
            7,
            &[CatalogOp::Rename {
                table: 1,
                name: "tickets".into(),
            }],
        )
        .unwrap();
        c.apply(9, &[CatalogOp::Drop { table: 2 }]).unwrap();
        assert!(c.as_of(2).is_empty());
        assert_eq!(c.as_of(3).len(), 2);
        assert_eq!(c.as_of(6).get(&1).map(String::as_str), Some("issues"));
        assert_eq!(c.as_of(7).get(&1).map(String::as_str), Some("tickets"));
        assert_eq!(c.id_of(8, "pages"), Some(2));
        assert_eq!(c.id_of(9, "pages"), None);
        assert_eq!(c.current().len(), 1);
    }

    #[test]
    fn ids_are_never_reused_and_names_are_unique_among_live_tables() {
        let mut c = Catalog::new();
        c.apply(1, &[create(1, "a")]).unwrap();
        assert_eq!(c.apply(2, &[create(1, "b")]), Err(CatalogError::IdUsed(1)));
        assert_eq!(
            c.apply(2, &[create(2, "a")]),
            Err(CatalogError::NameTaken("a".into()))
        );
        c.apply(2, &[CatalogOp::Drop { table: 1 }]).unwrap();
        assert_eq!(
            c.apply(3, &[create(1, "a")]),
            Err(CatalogError::IdUsed(1)),
            "a dropped id stays used"
        );
        c.apply(3, &[create(2, "a")]).unwrap();
    }

    #[test]
    fn a_failing_change_set_changes_nothing() {
        let mut c = Catalog::new();
        c.apply(1, &[create(1, "a")]).unwrap();
        let before = c.current();
        assert!(
            c.apply(2, &[create(2, "b"), CatalogOp::Drop { table: 9 }])
                .is_err()
        );
        assert_eq!(c.current(), before);
        assert_eq!(c.applied_lsn(), 1);
        assert_eq!(c.apply(1, &[create(5, "x")]), Err(CatalogError::OutOfOrder));
    }

    #[test]
    fn a_rename_may_swap_into_a_name_freed_in_the_same_change_set() {
        let mut c = Catalog::new();
        c.apply(1, &[create(1, "a"), create(2, "b")]).unwrap();
        // Free "a" first, then take it: valid because operations apply in order.
        c.apply(
            2,
            &[
                CatalogOp::Rename {
                    table: 1,
                    name: "tmp".into(),
                },
                CatalogOp::Rename {
                    table: 2,
                    name: "a".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(c.id_of(2, "a"), Some(2));
        assert_eq!(c.id_of(2, "tmp"), Some(1));
    }

    #[test]
    fn bad_names_and_payloads_are_refused() {
        assert_eq!(encode(&[create(1, "")]), Err(CatalogError::BadName));
        assert_eq!(
            encode(&[create(1, &"x".repeat(MAX_NAME + 1))]),
            Err(CatalogError::BadName)
        );
        assert_eq!(encode(&[]), Err(CatalogError::Empty));
        let mut bytes = encode(&[CatalogOp::Drop { table: 1 }]).unwrap();
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(CatalogError::Trailing));
        assert!(!is_catalog_payload(b"LROW"));
    }
}
