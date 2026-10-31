# ADR 0032: Copy-on-write branches as fork-point overlays

Status: proposed

Requirement P2-15 (`docs/spec/02-storage-and-sql.md`). Design: [RFC 0014](../rfc/0014-copy-on-write-branches.md), which is draft and has its own open questions. Issues #599 (this decision), #600 (reads and drops) and #601 (writes and conflicts). This ADR is proposed. It becomes a decision only when the owner accepts it; the existence of the pull request is not acceptance. It accepts no byte format, no phase gate and no performance claim.

## Context

P2-15 asks for branches with create, write, read and drop, a cost that does not depend on the size of the data, and tested isolation. The row store keeps every version in memory (`Table` in `crates/lattice-engines/src/mvcc.rs`), rebuilds it by replaying the log, and bounds it by vacuum. Vacuum is pinned only by open snapshots (`RowStore::vacuum_horizon`). Secondary indexes are derived from rows (ADR 0014). The change feed reads the main table's commit index. Old readers skip payloads they do not know (ADR 0022), so a change that alters what a read or a write means must be guarded by a required capability.

Copying the table at create is O(rows) and fails P2-15 directly. A shared version table keyed by branch changes every table operation and the main path. A persistent tree needs a new node format. Each of those is a larger decision than this one, and none is needed to meet P2-15 for the first version.

## Decision

1. **A branch is a record, a fork and an overlay.** The record names a parent (the main branch or a live branch). The fork is the LSN of the change set that created the branch. The overlay holds the versions written on the branch. The main branch has no fork and no overlay, so its code path and its API keep their meaning.
2. **Read rule.** A read of `(B, S)` takes the newest overlay version of the key with LSN at most `S`, if there is one. Otherwise it reads the parent at `min(S, fork(B))`, recursively. A tombstone means absent. The policy applies to every version returned, including parent versions.
3. **Create is one change set.** It writes the record and allocates an empty overlay. Nothing is copied. Its cost is independent of the rows.
4. **Writes go to the overlay.** A branch commit is checked for conflicts against its own overlay only. A parent write after the fork is not a conflict. Transactions are bound to one branch.
5. **Fork pins vacuum.** The horizon of a store may not exceed the smallest fork of a live branch that reads it. This is the only change to the vacuum rule.
6. **Drop frees the overlay and releases the pin.** Version 1 has no merge. A branch is read, written and dropped. A parent with live children is not dropped (RFC 0014, open question 5).
7. **Main-facing behaviour does not change.** Branch change sets produce no main change feed events, and they do not enter the main commit maps behind `AS OF` (RFC 0014, open question 7, with this as the proposal).
8. **The byte layout is deferred.** The format of a branch record and of a branch write, the catalog or payload choice, and the WAL capability bit are decided by a follow-up RFC and ADR. Until then, branch records and branch writes must be guarded by a required capability, so that a binary without branch support refuses the directory rather than vacuuming a version a branch still reads.

Alternatives not chosen, with RFC 0014 for the detail: a shared version table keyed by branch (changes the main path), a full copy at create (O(rows)), a persistent tree with structural sharing (a new node format), and git-style content addressing (needs the blob store of Phase 5).

## Consequences

- The main path is unchanged for a store that never creates a branch. The regression surface is the overlay and the fork pin.
- A read on a branch costs one ordered lookup per level of the parent chain. A scan merges the levels. The cost of a branch grows with its writes and its depth, not with the rows it did not touch. The depth bound is open (RFC 0014, open question 4).
- Overlays live in memory, as the main table does. There is no per-branch memory limit yet (open question 10). A forgotten branch holds its overlay and pins its parent's history until it is dropped. That is an operational risk, and drop must be tested with the vacuum floor.
- Checkpoints (RFC 0002, undecided) must carry branch records and overlays. Until that format is decided, branches cannot be checkpointed, and recovery replays them from the log.
- Format work follows: a byte-format RFC, a capability bit assignment, and an independent review of each. This ADR does not choose any of them.
- No gate, release or performance claim follows from this ADR. The benchmark named in #600 is a measurement to record, not a claim.
