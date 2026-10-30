//! The database the executor runs against: a row store, the catalog it persists and the column
//! definitions the executor keeps beside it.

use std::cell::RefCell;
use std::collections::BTreeMap;

use lattice_core::Config;
use lattice_core::bugs::{self, Bug};
use lattice_engines::catalog::{CatalogOp, DdlRef};
use lattice_engines::limits::{AllowAll, PolicyEvaluator};
use lattice_engines::sqldef::DefineTable;
use lattice_engines::store::RowStore;
use lattice_seams::{Clock, Io, Rng};
use lattice_sql::ast::CreateTable;

use crate::access::AccessPath;
use crate::definition;
use crate::error::{ErrorKind, ExecError};
use crate::exec::ActorPolicy;
use crate::schema::{ExecCatalog, MAX_TABLE_ID, Schema, TableDef, object_name};

/// A row store plus the schema the executor needs to read and write it.
///
/// Column definitions are durable (RFC 0012): `CREATE TABLE` commits the definition in the same
/// change set as the table, and [`Database::open`] rebuilds the schema from the definitions the
/// engine catalog recovered from the log. The schema here is a projection of that record, not a
/// second source of truth.
pub struct Database<I: Io, C: Clock + Clone, R: Rng> {
    pub(crate) store: RowStore<I, C, R>,
    pub(crate) clock: C,
    pub(crate) schema: Schema,
    next_rowid: RefCell<BTreeMap<u32, u64>>,
    /// How the scans of the last statement read their tables.
    access: RefCell<Vec<AccessPath>>,
}

impl<I: Io, C: Clock + Clone, R: Rng> Database<I, C, R> {
    /// Opens (or creates) the log, rebuilds the row store from it and rebuilds the schema from the
    /// table definitions the log carries. Refuses to open when a definition cannot be served.
    ///
    /// A table the catalog knows by name but that has no definition (it predates durable
    /// definitions) stays in the engine catalog and is not addressable by SQL.
    pub fn open(io: I, clock: C, rng: R, config: &Config) -> Result<Self, ExecError> {
        let store = RowStore::open(io, clock.clone(), rng, config)?;
        let mut schema = Schema::new();
        let names = store.catalog().current();
        for (id, record) in store.catalog().definitions() {
            let name = names.get(&id).ok_or_else(ExecError::corrupt)?;
            schema.insert(definition::from_record(id, name, record)?);
        }
        Ok(Database {
            store,
            clock,
            schema,
            next_rowid: RefCell::new(BTreeMap::new()),
            access: RefCell::new(Vec::new()),
        })
    }

    /// Allows the first table definition to upgrade a data directory that earlier builds can still
    /// open (RFC 0012). Off by default: such a directory refuses `CREATE TABLE` until the host sets
    /// this, because older builds cannot open the directory afterwards and there is no downgrade.
    /// A new, empty directory never needs it.
    pub fn set_allow_format_upgrade(&mut self, allow: bool) {
        self.store.set_allow_format_upgrade(allow);
    }

    /// How each scan of the most recent statement read its table, in execution order. Observability
    /// for tests and review: the choice never changes a result (see `access`).
    pub fn last_access_paths(&self) -> Vec<AccessPath> {
        self.access.borrow().clone()
    }

    pub(crate) fn clear_access(&self) {
        self.access.borrow_mut().clear();
    }

    pub(crate) fn record_access(&self, path: AccessPath) {
        let mut paths = self.access.borrow_mut();
        // A statement reads one table per scan and the subset has no joins; the cap keeps the
        // record bounded whatever a plan does.
        if paths.len() < 16 {
            paths.push(path);
        }
    }

    /// The underlying row store, for inspection and tests.
    pub fn store(&self) -> &RowStore<I, C, R> {
        &self.store
    }

    /// The column definitions known to the executor.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The catalog version plans are bound against. It grows whenever the engine catalog commits a
    /// change or a table is registered or removed here, so it differs after any change that could
    /// move a column position or a table id.
    pub fn catalog_version(&self) -> u64 {
        self.store
            .catalog()
            .applied_lsn()
            .saturating_add(self.schema.generation())
    }

    /// The binder's view of the schema at the current version.
    pub fn catalog_view(&self) -> ExecCatalog<'_> {
        ExecCatalog {
            schema: &self.schema,
            version: self.catalog_version(),
            actor: None,
        }
    }

    /// The id of the table called `name` in the engine catalog, as `policy`'s actor may read it.
    /// `None` for a table that does not exist and, identically, for any table the actor may not read
    /// (the whole catalog when the policy hides it), so a denied actor cannot tell the two apart. Tables whose columns SQL does not
    /// know (recovered from the log) are included: the change feed carries raw rows.
    pub fn visible_table_id(&self, policy: &ActorPolicy, name: &str) -> Option<u32> {
        if !policy.can_read_table(name) {
            return None;
        }
        self.store
            .catalog()
            .current()
            .into_iter()
            .find_map(|(id, n)| (n == name).then_some(id))
    }

    /// The binder's view for one actor: only the tables its policy lets it see (none when the policy
    /// hides the catalog).
    pub fn catalog_view_for<'a>(&'a self, policy: &'a ActorPolicy) -> ExecCatalog<'a> {
        ExecCatalog {
            actor: Some(policy),
            ..self.catalog_view()
        }
    }

    /// `policy` resolved against the catalog as it is now (see [`ActorPolicy::bound`]).
    fn bind_policy(&self, policy: &ActorPolicy) -> ActorPolicy {
        policy.bound(&self.store.catalog().current())
    }

    /// Creates a table: commits the name to the engine catalog and registers its columns.
    /// Returns `false` when the table exists and `if_not_exists` is set.
    ///
    /// The catalog change runs under `policy`: the engine asks it to authorize the DDL, and an actor
    /// without that grant gets `42501` before anything about the catalog is revealed.
    pub fn create_table(
        &mut self,
        ct: &CreateTable,
        policy: &ActorPolicy,
    ) -> Result<bool, ExecError> {
        let policy = &self.bind_policy(policy);
        let name = object_name(&ct.name)?;
        // Ask first, so a denied actor cannot probe which names exist. The ids here are placeholders:
        // the engine asks again with the real change at commit.
        authorize_ddl(
            policy,
            &[CatalogOp::Create {
                table: 0,
                name: name.clone(),
            }],
        )?;
        if self.schema.table(&name).is_some() {
            if ct.if_not_exists {
                return Ok(false);
            }
            return Err(relation_exists(&name));
        }
        let current = self.store.catalog().current();
        if current.values().any(|n| *n == name) {
            return Err(ExecError::new(
                ErrorKind::Bind,
                "42P07",
                format!(
                    "relation \"{name}\" exists in the catalog but has no stored definition, so SQL cannot address it"
                ),
            ));
        }
        // The engine catalog remembers every id ever created, live or dropped, so the next one is
        // free after any number of drops. Ids are never reused.
        let id = self
            .store
            .catalog()
            .next_table_id()
            .filter(|id| *id <= MAX_TABLE_ID)
            .ok_or_else(|| {
                ExecError::new(
                    ErrorKind::Limit,
                    "53400",
                    "too many tables: every table id the executor serves has been used",
                )
            })?;
        let def = TableDef::from_create(id, ct)?;
        // The definition is validated and encoded before anything is written, so a shape the record
        // cannot hold is a statement error and leaves no table behind.
        let record = definition::to_record(&def)?;
        // `IdUsed` cannot happen here: `&mut self` is the only writer and `id` is above every used
        // id. If it ever did, it would be an internal error (XX000), not something to retry.
        let tx = self
            .store
            .begin_with_policy(policy.clone())
            .with_author(policy.author());
        // The definition rides in the same change set as the table, so a crash leaves both or
        // neither and a recovered table always has its columns.
        let definitions = if bugs::active(Bug::ExecDefinitionNotPersisted) {
            Vec::new()
        } else {
            vec![DefineTable {
                table_id: id,
                definition: record,
            }]
        };
        self.store.commit_with_definitions(
            tx,
            vec![CatalogOp::Create {
                table: id,
                name: name.clone(),
            }],
            Vec::new(),
            definitions,
        )?;
        self.schema.insert(def);
        Ok(true)
    }

    /// Drops a table; its rows stay in the log. Returns `false` when it did not exist and
    /// `if_exists` is set.
    pub fn drop_table(
        &mut self,
        name: &str,
        if_exists: bool,
        policy: &ActorPolicy,
    ) -> Result<bool, ExecError> {
        let policy = &self.bind_policy(policy);
        let known = self.schema.table(name).map_or(0, |d| d.id);
        // Grants decide by name, so a name the schema does not know is judged exactly like one it
        // knows; deciding by the placeholder id would let a carved-out table's existence show.
        match policy.ddl_on_table(name) {
            Some(true) => {}
            Some(false) => return Err(ddl_denied()),
            None => authorize_ddl(policy, &[CatalogOp::Drop { table: known }])?,
        }
        let Some(def) = self.schema.table(name) else {
            if if_exists {
                return Ok(false);
            }
            return Err(ExecError::new(
                ErrorKind::Bind,
                "42P01",
                format!("table \"{name}\" does not exist"),
            ));
        };
        let id = def.id;
        let ops = vec![CatalogOp::Drop { table: id }];
        let tx = self
            .store
            .begin_with_policy(policy.clone())
            .with_author(policy.author());
        self.store.commit_with_catalog(tx, ops, Vec::new())?;
        self.schema.remove(name);
        self.next_rowid.borrow_mut().remove(&id);
        Ok(true)
    }

    /// The next row id for a table without a primary key. The counter starts after the largest key
    /// committed and is never reused within this process, even by a rolled back transaction.
    pub(crate) fn alloc_rowid(&self, table: u32) -> Result<u64, ExecError> {
        let mut ids = self.next_rowid.borrow_mut();
        let next = match ids.get(&table) {
            Some(n) => *n,
            None => {
                let snap = self.store.snapshot();
                let rows = self.store.scan_at(snap, table, .., &AllowAll)?;
                rows.last()
                    .and_then(|(k, _)| <[u8; 8]>::try_from(*k).ok())
                    .map_or(1, |k| u64::from_be_bytes(k).saturating_add(1))
            }
        };
        ids.insert(table, next.saturating_add(1));
        Ok(next)
    }
}

/// Asks the actor's policy whether it may run this catalog change.
fn authorize_ddl(policy: &ActorPolicy, tables: &[CatalogOp]) -> Result<(), ExecError> {
    if policy.authorize_ddl(DdlRef {
        tables,
        indexes: &[],
        definitions: &[],
    }) {
        Ok(())
    } else {
        Err(ddl_denied())
    }
}

fn ddl_denied() -> ExecError {
    ExecError::new(
        ErrorKind::Permission,
        "42501",
        "permission denied: the actor may not change the catalog",
    )
}

fn relation_exists(name: &str) -> ExecError {
    ExecError::new(
        ErrorKind::Bind,
        "42P07",
        format!("relation \"{name}\" already exists"),
    )
}
