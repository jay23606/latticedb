# RFC 0013: Live queries as incremental views

Status: draft
Comment period ends: 2026-11-07
Decision ADR: none yet

This RFC defines the semantics of a live query: which queries are in the first class, what a view must equal,
how it is maintained from the change feed, how it starts, and how each failure is reported. It changes no
byte on disk and none on the wire. Four choices are format or wire decisions and are left open below
(section "Open questions"); each needs its own RFC, ADR and independent review before code depends on it.

Requirement P3-03 (`docs/spec/03-sync-and-sdk.md`). Work package WP41 (issue #243). Step tracked in issue #627.

## Motivation

Phase 3 requires that a live query is an incremental view over the change feed, for a defined query class, and
that its result equals re-running the query at the same snapshot. WP41 adds three conditions: the class is
documented, everything else is refused clearly, and a planted missed-delete bug is caught.

The first product slice (`docs/design/live-collaboration.md`) needs a reader to watch the rows of a table as
they change. Today a reader can subscribe to a whole table (`table/<name>`) and must filter client side. A
filter or projection added without a written definition would leave no way to say whether a live result is
correct. This RFC supplies that definition before any code.

### Code facts this proposal relies on

All paths are on `origin/main` at `ffa2f9de`.

1. **The feed is paged and table-filterable.** `RowStore::read_feed_as(cursor, table, policy, limits)` in
   `crates/lattice-engines/src/feed_emit.rs` returns a bounded `FeedPage { events, resume, more, scanned }`.
   With `table` set, other tables are skipped without a policy check or an allocation. The scan counter
   (`scanned`) still counts their versions, so the budget applies to them too.
2. **An event does not carry the previous value.** `ChangeEvent` (`crates/lattice-core/src/feed.rs`) holds a
   cursor `(lsn, index)`, `set_len`, the provenance and an `EntityChange { kind, key, op }`, where `op` is
   `Delete` or `Upsert(bytes)`. The bytes of an upsert are a typed row (`decode_row` in
   `crates/lattice-engines/src/typed.rs`, used by `crates/lattice-api/src/feed_source.rs`). `formats/change-feed-v1.md`
   gives the encoding.
3. **The policy filters events, not row transitions.** `read_feed_as` judges an upsert by its new value and a
   delete by its previous value (`entry.value.or(entry.previous)`). A row that becomes unreadable produces no
   event for that reader. The module header of `feed_emit.rs` says so, and threat model row G-07
   (`docs/threat-model.md`) records it. A reader that keeps state cannot learn that the row left.
4. **Snapshots are LSNs.** `Snapshot { as_of }` (`crates/lattice-engines/src/mvcc.rs`) sees every change set
   applied at or before `as_of`. `RowStore::scan_at(snap, table, range, policy)` and `get_at` apply the policy to
   the rows they return (`crates/lattice-engines/src/store.rs`). `check_snapshot` fails with `SnapshotTooOld`
   below the vacuum floor.
5. **The floor refuses old cursors.** A feed read at or below `oldest_snapshot().as_of` fails with
   `CursorTooOld` (`read_feed_as`). `FeedSource` maps that to `SourceError::TooOld`.
6. **Oversize keys are skipped.** A row whose key is longer than `MAX_KEY_BYTES` can exist in a log written before
   the bound. The feed module headers say a reader is moved past that event. A stream loses nothing that way,
   but a view that skipped it would silently lack that row.
7. **A kernel-backed `EventSource` exists.** `FeedSource` (`crates/lattice-api/src/feed_source.rs`) implements
   `EventSource` with a scan budget (`DEFAULT_SCAN_BUDGET` 4096, `MAX_ROUNDS` 1024). Its own module header
   records a known limit: the scan is not per table, and a quiet table can get `Unavailable` after about 4.2
   million versions of other activity.
8. **A commit notifier exists.** `CommitNotifier::wait_after` (`crates/lattice-api/src/notify.rs`) backs the
   `lattice_subscribe` long poll in `crates/latticed/src/mcp_backend.rs`.
9. **Snapshot-then-tail is not built.** `feed_source.rs` says reconnecting through a snapshot or bootstrap path
   is issue #525 and "not built". No code in the repository starts a subscription from a snapshot.
10. **Two cursor forms.** The sync protocol carries an opaque cursor of 1 to 64 bytes
    (`formats/sync-protocol-v1.md`). `FeedSource` and the MCP tools use the 12-byte feed cursor
    (`lsn u64 | index u32`, big endian). Sync errors `invalid_scope` (4) and `invalid_cursor` (5) are defined there.
11. **Grants are per table.** `PolicyTable` (`crates/lattice-api/src/exec_host.rs`) grants read, write and ddl
    per table. There are no row predicates in the policy today (threat model G-01).
12. **Planted feed bugs exist; none covers views.** `FeedLimitIgnored`, `FeedScanBudgetIgnored` and
    `FeedIndexCommitSkipped` are in `crates/lattice-core/src/bugs.rs`. `FeedSourcePolicyDropped` covers the
    source's policy. No bug covers view maintenance.

`docs/design/live-collaboration.md` lists the kernel-backed event source (#519) and the commit notifier (#523)
as missing. Facts 7 and 8 show both exist on `main`. That note is not edited here. Its owners should update it.

## Proposal

### 1. The query class

A live query has four parts. Nothing else is in class version 1.

* **Source.** Exactly one table `T`, named by the scope `table/<name>` that the feed already serves.
* **Filter.** Optional. A boolean expression over the columns of `T`, built from: comparisons `=`, `<>`, `<`,
  `<=`, `>`, `>=` between a column and a literal; `IS [NOT] NULL` on a column; `[NOT] IN (literal list)`;
  `[NOT] BETWEEN` with literal bounds; combined with `AND`, `OR`, `NOT` and parentheses. These are the `WHERE`
  forms in `docs/sql-subset.md`. Excluded: `LIKE`, parameters, subqueries, functions, arithmetic, `CASE`.
* **Projection.** `*` or a list of distinct column names of `T`. No expressions.
* **Identity.** A row is identified by its key in `T` (`EntityChange.key`). The view is a set of rows under that
  key. The view has no order of its own; a client that wants an order sorts locally.

Refused, each with a named reason: `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT`, `GROUP BY`, aggregates, joins,
subqueries, more than one table, `LIKE`, parameters, expressions in the projection.

A literal compared with a column must have a type the column accepts. A mismatch is refused at subscribe time,
before any row is read. Arithmetic is excluded, so the class has no row-time evaluation error.

A row is in the view when the filter is TRUE. NULL and FALSE exclude it, as in `WHERE`.

Refusal order: scope visibility first, then the class. A principal without read on `T` gets the same answer as for
a table that does not exist (the rule in `feed_source.rs`). A principal with read gets a refusal that names the
construct.

### 2. What the view must equal

For a principal `P` with policy `π`, a query `Q` over `T`, and a snapshot `S_L` with `as_of = L`:

    Fresh(P, Q, L) = { (k, project(v)) : (k, v) in scan_at(S_L, T, all, π) and filter(v) is TRUE }

The view `V` must equal `Fresh(P, Q, L)` after every complete change set at LSN `L` that it has applied.

### 3. Maintenance rule

The maintainer keeps `V`, a map from key to projected row. It reads the events of `T` in cursor order with
`read_feed_as(cursor, Some(T), AllowAll, limits)`. This is trusted host code; no output leaves it without `π`.
For each event with key `k`:

* **Upsert `v`.** If `π` allows reading `(T, k, v)` and `filter(v)` is TRUE, set `V[k] = project(v)`, inserting or
  replacing. Otherwise remove `V[k]` if it is present.
* **Delete.** Remove `V[k]` if it is present.

The rule needs no previous value, because `V` records membership. It applies `π` to the new value on every upsert,
so a row that stops being readable or stops matching leaves `V`. That is the case the policy-filtered feed cannot
express (fact 3).

**Atomicity.** The view publishes a new state only after the event with `index = set_len - 1` of a change set has
been applied (`formats/change-feed-v1.md`). A client never sees part of a transaction. The feed emits one event per
key per change set (last write wins, `feed_emit.rs`), so the order inside a set does not change the result.

**Invariant.** After each complete change set at `L`, `V = Fresh(P, Q, L)`. A key the set does not touch keeps its
membership. A key it touches has its membership recomputed from its final value at `L`.

**Cost.** Per commit, the maintainer examines the versions after its resume cursor. The table filter skips other
tables without policy checks or allocation, but their versions still count toward the scan budget. Cost therefore
grows with the commits after the cursor, not with the size of `T`. The maintainer keeps its own resume cursor and
continues across steps. It has no client waiting, so `MAX_ROUNDS` does not apply to it.

**Sharing.** One maintainer serves one (principal, query) pair in version 1. Sharing one feed read among principals
is a later optimisation (design note, risk 2).

**Memory.** `V` holds only rows the principal may read and that match. Its size is capped by a row limit (open
question 7).

### 4. Snapshot, then tail

1. Take `S = store.snapshot()` at `L`.
2. Send `Fresh(P, Q, L)` as `enter` messages in key order (`scan_at` with `π`), then a `ready` marker that names `L`.
3. Tail from cursor `(L, u32::MAX)`, which is strictly after every event of commit `L`. Apply events with `lsn > L`
   by the rule in section 3.

If the vacuum floor passes `L` before step 3 reads, the read fails with `CursorTooOld`. The view is then rebuilt from
step 1. The design note records this race ("a cursor token for a snapshot can fall below the floor before the
subscribe arrives").

### 5. Output

A client receives changes to `V` only: `enter` (key, projected row), `update` (key, projected row) and `leave` (key),
each with the feed cursor of the event that caused it, and the `ready` marker of step 2. A `reset` (the cursor of the
event that caused it, and a reason: `ddl` or `policy`) ends the view: nothing after it is valid for that view, and the
client must re-snapshot (section 6). A write that does not change `V` sends nothing. The encoding of these messages is
not defined here (open question 2).

### 6. Failure semantics

| Case | Result | What the client does |
|---|---|---|
| Scope not visible to `P`, or unknown | Same answer for both: `UnknownScope`, sync `invalid_scope` | Nothing |
| Query outside the class | Refused before any event; the reply names the construct | Change the query |
| Cursor at or below the floor (`TooOld`) | `invalid_cursor` | Re-snapshot (section 4) |
| Cursor never issued (`BadCursor`) | `invalid_cursor` | Re-snapshot |
| Feed step `Unavailable` (budget, shutdown) | The view keeps its last complete state and cursor and does not advance. It must not report itself as caught up. Retries continue from the last cursor | Reconnect from the last acknowledged cursor |
| `IndexInconsistent` or `MissingCommitTime` | Terminal for the view until reopen or repair (`feed_source.rs`) | Operator action |
| Oversize key in a legacy log (`KeyTooLarge`) | The view enters a failed state that names its cursor. It must not skip the event; skipping is correct for a stream and silently wrong for a view | Operator repair |
| DDL on `T`, or a change to `π` for `P` | The view publishes `reset` and stops. It never continues silently under new rules | Re-snapshot |
| Subscriber too slow | Backpressure (P3-14): the subscriber is disconnected with its resume cursor. View state is unaffected | Reconnect |
| Process restart | The view is rebuilt from a snapshot. The feed is durable (`feed_emit.rs`, "Resume"), so a feed cursor above the floor still resumes | Re-snapshot if `TooOld` |

Runtime predicate errors cannot occur in version 1: there is no arithmetic, and literal types are checked at
subscribe time.

### 7. Non-goals

Joins, aggregates, `GROUP BY`, ordering with `LIMIT`, `DISTINCT`, subqueries, multi-table views, parameters, `LIKE`,
computed projections, presence (#533), collaborative documents (#532), workflow transitions (#531), a write frame in
the sync protocol, QUIC, multi-node sync (Phase 10), and any gate or release claim.

The issue lists "SQL subset stable" as a dependency. The subset is still moving: open pull requests #729 (`greatest`
and `least`), #730 (`CASE`), #732 (string functions), #733 (`ILIKE`), #751 (ungrouped aggregates) and #753
(`GROUP BY`) add to it. Version 1 excludes all of them and uses only the `WHERE` forms already listed in
`docs/sql-subset.md`.

## Alternatives

* **A. Re-run the query on every commit.** Simple and obviously correct, at a cost proportional to `T` for each view
  and each commit. It is the test oracle (section "Test sketch") and not the serving path.
* **B. Emit leave events in the feed.** The feed would then carry the previous value or a leave kind. That changes
  the change-feed bytes, which are still in RFC 0009 draft. Rejected for version 1; the membership rule does not
  need it.
* **C. Serve views from the policy-filtered feed (`FeedSource` as it is).** Rejected. It cannot emit leave events
  (fact 3), and its skip rule for oversize keys is wrong for a view (section 6).
* **D. Adopt an external incremental-view engine.** Rejected for version 1. It adds a dependency to a layered,
  deterministic engine, and the class is small enough for the membership rule. Revisit when joins or aggregates
  are scheduled.
* **E. Joins and aggregates in version 1.** Rejected. The membership rule does not extend to them without per-operator
  state. They need their own RFC.

## Compatibility and migration

* No byte on disk or on the wire changes. The change-feed format, the sync v1 frames and the MCP tool schemas stay as
  they are.
* `table/<name>` keeps its meaning: the unfiltered table view.
* `FeedSource` and the MCP subscribe tool keep their behaviour.
* How a query reaches the server is open (question 1). Any choice that changes `Subscribe`, the scope grammar or an MCP
  schema is a wire change. It needs an RFC 0010 amendment while that RFC is still draft, and an ADR.
* No migration. Views live in memory and are rebuilt from the log.

## Open questions

1. **Wire form of a query.** Sync v1 `Subscribe` carries a scope text and an opaque cursor. MCP `lattice_subscribe`
   carries a scope, a table, `after` and `wait_ms`. A filter and a projection need either an extended scope grammar
   (the server defines scope syntax, design note decision 2) or a `Subscribe` variant that carries a query. Both
   change a wire surface.
2. **Encoding of view changes.** `enter`, `update`, `leave` and `ready` need bytes. This is a new format. It needs an
   RFC, an ADR and an independent review.
3. **Cursor of a view.** Using the feed cursor leaks LSN gaps across tables and index gaps (G-07). A dense per-view
   sequence hides them, but it must be persisted to resume after a restart (#528). This is an owner decision.
4. **Filter evaluator.** `lattice-exec`'s evaluator (`eval` in `crates/lattice-exec/src/eval.rs`) is `pub(crate)` and
   consumes the planner's typed expressions. ADR 0025 (in-house planner) is accepted, and ADR 0013 is proposed. Sharing
   the executor's evaluator makes the view and the fresh query use the same semantics, which the equality property
   needs. The test oracle must still be a separate reference evaluator. Which crate hosts the maintainer is open
   (layering).
5. **Store key and SQL primary key.** Whether a SQL row's feed key is exactly its primary-key encoding is not yet
   verified. Check `crates/lattice-exec/src/schema.rs` before implementation.
6. **Tail cursor.** Confirm that the source's `BadCursor` check accepts `(L, u32::MAX)` as a start. Snapshot bootstrap
   (#525) is not built.
7. **Limits.** The view row cap and the per-principal budget (#529). No value is proposed here.
8. **Change detection.** How a change to `PolicyTable` or a DDL event reaches the maintainer is not designed.
9. **Row-level policy.** When row predicates arrive (Phase 4), leaves happen on ordinary writes. The rule in section 3
   already covers this. The cost of evaluating `π` per upsert grows and needs measurement.
10. **Cursor width.** Whether view cursors use the 12-byte feed form or a longer one is not decided.

## Test sketch

These are acceptance criteria for the implementation. They are not part of this RFC.

* **Property `view_equals_fresh_query_after_random_changes` (P).** Random change sets over a small key domain: several
  writes per set, deletes, a key written twice in one set, aborted transactions, rows moving into and out of the
  filter and the policy. After each complete change set at `L`, `V` equals the reference `Fresh(P, Q, L)`.
  `PROPTEST_CASES` is 10,000 on pull requests (`AGENTS.md`, section 2).
* **Reference oracle.** The reference uses a separate evaluator written from section 1. Expected values are not computed
  by the code under test.
* **Resume (S).** A client that stops at cursor `c` and resumes by snapshot then tail sees no gap and no duplicate,
  including after restarts. Run under the `lattice-sim` fault profiles with reordered and duplicated delivery and
  reopen mid-stream.
* **Planted bugs.** Each must make the property fail, and the clean build must pass. These names are proposals and are
  added with the code:
  * `LiveViewDeleteIgnored`: deletes are dropped. This is the missed-delete case of WP41.
  * `LiveViewLeaveIgnored`: a row whose new value is no longer readable or matching is kept.
  * `LiveViewPartialSetPublished`: state is published before the event with `set_len - 1`.
* **Refusal matrix (T).** Unknown table, denied table, each unsupported construct and a literal type mismatch get the
  answers of section 6. A denied principal and an unknown table get identical answers.

## Review

This draft needs an independent read, by a reviewer who did not write it, before it leaves draft. The `reviewed`
label is not claimed by this RFC.
