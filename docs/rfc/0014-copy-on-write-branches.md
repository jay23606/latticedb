# RFC 0014: Copy-on-write branches

Status: draft
Comment period ends: 2026-11-07
Decision ADR: none yet (ADR 0032 is proposed in the same pull request; it becomes the decision only when this RFC is accepted)

This RFC defines copy-on-write branches for the row engine: what a branch is, what a read, a write, a drop and a vacuum mean on one, what each operation costs, and what it requires of the log, recovery and the change feed. It changes no byte of any format today. The byte layouts that branches need are not defined here (open question 1). They need their own RFC, ADR and independent review before code depends on them.

Requirement P2-15 (`docs/spec/02-storage-and-sql.md`): create, write, read, drop; cost independent of data size; branch isolation tested. Issues #599 (this RFC), #600 (reads and drops) and #601 (writes and conflicts) implement it. This RFC makes no Phase 2 gate claim.

## Motivation

Phase 2 needs branches for forks, subagent workspaces and review copies. `docs/ARCHITECTURE.md` (section 6, the "Copy-on-write branches" row of the design table) says these "are branch pointers, not copies". Nothing in the repository implements a branch today. `formats/isolation.md` already says a snapshot is `(branch, S)`, and lists branches as not implemented.

Implementation work in #600 and #601 cannot be tested against an oracle until the repository says what a branch read must equal, what a branch write may change, and which operations must not depend on the size of the data. This RFC supplies that definition before any code.

### Code facts this proposal relies on

All paths are on `origin/main` at `29c60ec4`.

1. **Versions are the unit of MVCC.** `Table<K, V>` (`crates/lattice-engines/src/mvcc.rs`) keeps, for each key, a list of versions ordered by LSN. `Snapshot { as_of }` in the same file is a plain LSN. A read at a snapshot takes the newest version at or before `as_of` (`visible`).
2. **A commit is one change set at one LSN.** `RowStore::commit` (`crates/lattice-engines/src/store.rs`) authorizes the writes, runs the conflict check (`Tx::check`), appends through the kernel, then applies the writes to the table and to the indexes at the commit LSN. The log is the only durable copy. `RowStore::open_with_indexes` rebuilds every row version by replaying the row change sets.
3. **The row payload has no branch field.** `LROW` version 1 (`formats/rowops-v1.md`, `crates/lattice-engines/src/rowops.rs`) carries a table id, a key and a put or delete. A change set (`formats/changeset-v1.md`) carries an id, an HLC, an author, optional turn fields, parent ids and one payload.
4. **Vacuum is pinned only by open snapshots.** `RowStore::vacuum_horizon(oldest_open, retain_lsns)` takes the smaller of the oldest open snapshot and the retention window. `Table::vacuum` clamps the horizon to `[floor, applied]`. Nothing else keeps a version alive, so a fork point would need its own pin.
5. **Secondary indexes are derived from rows** and versioned at the same LSN (ADR 0014, `crates/lattice-engines/src/index.rs`). A branch must say what happens to index entries.
6. **The change feed reads the main table's commit index.** `read_feed_as` and `events_since_as` (`crates/lattice-engines/src/feed_emit.rs`) walk `commits_from` of the row table. Whatever a branch writes must not reach that walk unless the proposal says so.
7. **Old readers skip unknown payloads.** ADR 0022 records that old RowStore readers skip unknown engine payloads, and that a change they must refuse needs a required capability in the WAL header (RFC 0008). `docs/gates/phase-2.md` says durable table definitions use capability bit 3. `formats/wal-admission-v2.md` lists bit 2 (value 4) as unassigned at this head.
8. **Catalog changes are change sets.** `formats/catalog-v1.md` (draft) gives table ids that are never reused and names unique among live tables. Its payload magic is `LCAT`.
9. **Policy is per table.** `PolicyTable` in `crates/lattice-api/src/exec_host.rs` maps each account to a `Grant`. `Grant::Tables` gives read, write and ddl bits on named tables, and a table without a bit is invisible. There is no branch permission. Per-table grant decisions for tokens are issue #522.
10. **No branch type exists.** `git grep -i branch` over `crates/` finds no branch concept in the engines or the SQL layer. `docs/gates/phase-2.md` lists SQL `AS OF` and branches as not on `main` (#598 to #601).

## Proposal

### 1. Model

- A **branch** `B` is a record with an id (never reused), a name (unique among live branches), a parent `P` (the main branch or another live branch), a **fork** (an LSN: the commit LSN of the change set that created it), and an **overlay**: the row versions written on `B`.
- The **main branch** is the table as it is today. It has no fork and no overlay. Every existing API keeps its meaning on main.
- LSNs are the LSNs of the one log. The fork and every branch snapshot are comparable with main's.
- A branch snapshot is `(B, S)`. To read key `k` of table `T` at `(B, S)`:
  1. If the overlay of `B` has a version of `k` with LSN `<= S`, the newest such version decides. A tombstone means absent.
  2. Otherwise read the parent at snapshot `min(S, fork(B))`, by the same rule, recursively, until the main branch.
- In words: a branch sees its parent exactly as the parent was at the fork, plus its own change sets up to `S`. A parent write after the fork is never seen by the branch. A branch write is never seen by the parent.

### 2. Operations

- **Create** `B` from `P` at the applied LSN. It is one change set that writes the branch record (I2: visible in the log and in every engine, or in neither). Its LSN is `fork(B)`. The name must be free among live branches. The overlay starts empty. Nothing is copied from the parent.
- **Read** (point read, range scan, index lookup) at `(B, S)`: by the rule in section 1. The policy `P` of the transaction applies to every version returned, whether it comes from the overlay or from the parent (P2-17). A fall-through to the parent is a read of the parent, not a bypass.
- **Write**: a transaction is bound to one branch. Cross-branch transactions are refused (`formats/isolation.md`, open questions). Its writes are buffered and published as one change set, as on main. The change set names its branch (open question 1). The writes go to the overlay of that branch only.
- **Conflict rule** (first committer wins, `formats/isolation.md`): the check for a branch commit looks only at the overlay of that branch, for versions with LSN greater than the transaction's `S`. A parent version written after the fork is not a conflict, because the branch never read a later parent version. A blind write (put without a read) is judged the same way.
- **Drop** `B`: removes the record and frees its overlay. The fork no longer pins the parent (section 2, vacuum). A snapshot or transaction bound to a dropped branch fails at its next read or commit with a branch error (open question 6). Drop is refused for a parent that has live children (open question 5).
- **Merge** of a branch into its parent is not part of version 1. A branch is read, written, and dropped. Nothing makes its writes visible to the parent. The `Merging` state in `docs/ARCHITECTURE.md` (section 16.4) is a later design.
- **Vacuum**: the horizon of a store must not exceed the smallest fork of any live branch that reads it, directly or through a chain. A fork is a pin in addition to the open snapshots that `vacuum_horizon` already takes. Each overlay is vacuumed to its own floor by the rule of `Table::vacuum`. A horizon that is clamped by a fork keeps the parent version visible at that fork, so every live branch still reads what it read before.
- **AS OF**: `AS OF CHANGESET` and `AS OF TIMESTAMP` on main keep their meaning. On a branch they name `(B, S)`. How branch change sets enter the main commit maps (`commit_lsn`, `commit_times`) is open question 7.
- **Change feed**: the main feed is unchanged. Branch change sets produce no main feed events. A feed for a branch is not part of version 1 (open question 8).
- **Recovery**: branch create and drop records and branch writes are replayed in LSN order, as main commits are. Overlays are rebuilt by the replay. Opening a store costs O(history), as it does today. No checkpoint is defined here (open question 9).

### 3. Cost

`d` is the depth of the branch chain (main has depth 0). `n` is the number of keys in one overlay or table. `v` is the number of versions of one key.

| Operation | Cost | Depends on the number of rows in the table? |
|---|---|---|
| Create `B` | O(1): one change set and an empty overlay. Nothing is read from or copied from the parent | No |
| Point read at `(B, S)` | O(d log n + d log v): one ordered lookup per level of the chain | No |
| Range scan at `(B, S)` | O(rows in the range x d): a merge over `d + 1` ordered sources | Only rows read, as on main |
| Write one key on `B` | O(log n + log v), plus index maintenance (ADR 0014) | No |
| Commit of `w` writes on `B` | O(w (log n + log v)) with the conflict check on the overlay only | No |
| Drop `B` | O(size of its overlay) to free; the fork pin is released at the next vacuum | Only the overlay |
| Write on main after a fork | Same as today | No |
| Vacuum of main | O(versions removed); the horizon may be clamped lower by a fork | No |
| Open the store | O(history), replay of every row change set, as today | Yes, as today |

The creation cost does not grow with the rows. That is the requirement of P2-15 and the benchmark named in #600. Reads and scans on a branch still cost what they cost on main, plus one ordered source per level of the chain. The cost of a branch therefore grows with what it writes and with its depth, not with the size of the data it did not touch.

### 4. Secondary indexes

A branch overlay holds the index entries of its own rows, derived by the rule of ADR 0014. An indexed lookup on `(B, S)` must return what a scan of `(B, S)` with the same predicate returns. That is the invariant. Version 1 may instead fall back to a scan of the branch for indexed predicates (open question 3). Either way, a check like `check_indexes_at` must run per branch.

### 5. Policy and grants

Every version returned on a branch is judged by the transaction's `PolicyEvaluator`, including versions that come from the parent. Who may create, read, write or drop a branch is open (question 2). The proposal until that is decided: a branch operation requires the same table grant as the matching table operation, and a branch is visible only to the account that created it.

## Alternatives

- **A. Fork-point overlay per branch (proposed).** A branch is a record, a fork LSN and an overlay. Create is O(1). The main path is unchanged.
- **B. One shared version table with a branch id in each key.** Create is also O(1). Every table operation, the commit index and vacuum change for all data, including main. Rejected for version 1 because it changes the main path that all existing tests cover. It can be revisited if many small overlays cost too much.
- **C. Copy the parent at create.** O(rows) per branch. Rejected: P2-15 requires cost independent of data size.
- **D. Persistent tree with structural sharing.** O(1) create and O(log n) copy-on-write per write. Rejected for version 1: it needs a new node format, which is a larger format decision. It is a candidate if measured depth or overlay memory hurts.
- **E. Use the ChangeSet `parents` field as the branch structure.** That field records causal order between change sets. It does not name a branch head or a fork, so it cannot say what a branch reads. Rejected.
- **F. Content-addressed snapshots (git-style).** Needs a content store. That belongs to blob storage (Phase 5). Rejected for Phase 2.

## Compatibility and migration

- **Main is unchanged.** No byte on disk changes for a store that never creates a branch. `Snapshot` keeps its meaning. A branch snapshot is a new value, `(B, S)`.
- **Old binaries must refuse, not skip.** A binary that predates branches does not know the fork pin. If it vacuums the parent, it can remove a version a live branch still reads. Branch records and branch writes must therefore sit behind a required capability (ADR 0022 and RFC 0008), so an older reader refuses the directory instead of skipping the records. The new bit is an open question (question 1).
- **WAL framing** (`formats/wal-v1.md`) is not changed by this RFC.
- **Change feed** (`formats/change-feed-v1.md`) is not changed for main.
- **Checkpoints** are not defined. A checkpoint must carry branch records and overlays. The `Table` commit-index comment in `mvcc.rs` already warns that a restore must rebuild its indexes. RFC 0002 decides the format and is open.

## Open questions

1. **Byte format.** How a branch record and a branch write are encoded: a new `LROW` version or a new magic for branch writes, a catalog operation (`formats/catalog-v1.md` has operation kinds 1 to 3) or a separate record, and which WAL capability bit (bit 2 is unassigned at this head). Each needs its own RFC and ADR. This RFC does not fix any of them.
2. **Permissions.** Whether a branch needs its own grant, or reuses the table grant for each operation, and whether a branch is visible to other accounts. This depends on per-table grants (#522).
3. **Indexes on branches.** Whether version 1 maintains overlay index entries, or falls back to a scan for indexed predicates on a branch. The result must be identical in both cases.
4. **Depth.** The maximum depth of a branch of a branch. The proposal needs a bound, but no value is chosen here. It should come from a measurement of the fall-through cost in section 3.
5. **Parents with children.** Whether a parent with live children may be dropped or altered. The proposal refuses DROP TABLE and DDL on any table that a live branch reads, and refuses dropping a branch that has children. The alternatives are to reparent, or to materialize the overlay, which costs O(rows). The owner decides.
6. **Errors.** The stable error name and SQLSTATE for a dropped branch, and for a branch snapshot that falls below the parent's floor. The SQL layer owns the SQLSTATE mapping (#598 to #601).
7. **Commit maps.** Whether branch change sets enter `commit_lsn` and `commit_times` (the maps behind `AS OF` on main). The proposal is that they do not, so that main `AS OF` keeps meaning "a main commit".
8. **Branch feed.** Whether a branch needs a change feed. Live views (RFC 0013) read the main table only.
9. **Checkpoint.** The checkpoint format (RFC 0002) is undecided. No checkpoint can carry branches until it is.
10. **Memory limits.** Per-branch memory and the number of live branches, under the per-query and per-engine limits of P2-18. No value is proposed.
11. **Snapshots below the floor.** Whether a branch snapshot below the parent's vacuum floor is refused with `snapshot too old`. The proposal refuses it, as on main.

## Test sketch

These are acceptance criteria for #600 and #601. They are not part of this RFC.

- **Model.** A model of each branch is a map copied from the parent at the fork in the test, not computed by the code under test. Random sequences of create, write, read, drop, vacuum, crash and recover, checked against that model. Expectations come from the model, not from running the implementation.
- **Read equality.** `branch_reads_equal_parent_at_fork_point_for_every_table`: a branch read equals the parent read at the fork, plus the branch's own writes up to `S`.
- **Isolation.** `branch_write_is_invisible_to_parent_until_merged_or_never`: parent writes after the fork are not seen by the branch, and branch writes are not seen by the parent.
- **Drop.** `drop_branch_releases_versions`: after drop and vacuum, the overlay is gone and the fork no longer clamps the horizon.
- **Cost.** `branch_create_time_does_not_grow_with_rows`, over three table sizes, recorded in the gate page. It is a measurement, not a claim.
- **Planted bugs.** Each must make a test fail while the clean build passes. Proposed names, added with the code: `BranchSharesMutableVersion` (named in #600), `BranchReadsLiveParent` (reads the parent at `S`, not `min(S, fork)`), `BranchVacuumIgnoresFork` (the fork does not clamp the horizon), `BranchWriteLeaksToParent`, `BranchConflictWithParent` (the conflict check reads the parent), `BranchFeedLeaksIntoMain`.
- **Crash.** The crash sweep includes create and drop of a branch, with a reopen in between.

## Review

This draft needs an independent read, by a reviewer who did not write it, before it leaves draft. The `reviewed` label is not claimed by this RFC.
