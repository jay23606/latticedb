# Phase 2: storage engines, MVCC and SQL over the PostgreSQL wire

## Goal

Turn the Phase 1 log into a queryable database. Add the row and columnar engine, the catalog, MVCC with branches and time travel, the unified WAL across engines, and a documented SQL subset reachable by standard Postgres drivers.

## Scope

**In:** catalog, row/columnar engine with facet tables, MVCC keyed by ChangeSet, snapshot isolation, secondary indexes, unified WAL across log and row engines, checkpoints, SQL parser, planner and executor for the declared subset, PostgreSQL wire v3 (simple and extended query, TLS), `AS OF` queries, copy-on-write branches, online schema change with dual-write.

**Out:** sync, identity (policy stays allow-all behind the real trait), blobs, search, graph.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P2-01 | Catalog persisted through the log; catalog changes are ChangeSets; catalog is versioned and time-travelable | S, T |
| P2-02 | Each LSL facet type compiles to a typed physical table plus overflow column; no generic EAV table | T |
| P2-03 | MVCC where a row version is a ChangeSet id; readers use immutable snapshots and never block writers | S, P |
| P2-04 | Snapshot isolation with documented anomalies; write-write conflict detection per row; behaviour specified in `formats/isolation.md` | S, P |
| P2-05 | **I2 across log and row engines**: a ChangeSet is fully visible in both or neither, under crash and fault injection | S |
| P2-06 | **I3**: no reader ever observes a partially applied ChangeSet | S, P |
| P2-07 | Secondary indexes maintained transactionally; **I9** scrubber rebuilds each index from the log and diffs | S, T |
| P2-08 | Checkpoints bound recovery time; recovery time is measured against data size and reported | B, S |
| P2-09 | Declared SQL subset in `docs/sql-subset.md`: DDL derived from LSL, SELECT with joins, filters, aggregates, ordering, limits, INSERT, UPDATE, DELETE, transactions, prepared statements | C |
| P2-10 | Planner and executor chosen by ADR (custom versus `sqlparser` plus DataFusion) with measured criteria | I, B |
| P2-11 | PostgreSQL wire v3: startup, authentication hook, simple and extended query, binary and text formats, TLS, cancel requests | C, T |
| P2-12 | Driver matrix passes: `psycopg`, `pgx`, JDBC, `tokio-postgres`, `node-postgres` on a defined test script | C |
| P2-13 | `sqllogictest` suite for the declared subset passes 100 percent of declared tests; unsupported syntax fails with a clear, stable error code | C |
| P2-14 | `AS OF` change set or timestamp queries over any table | T, P |
| P2-15 | Copy-on-write branches: create, write, read, drop; cost independent of data size; branch isolation tested | S, P |
| P2-16 | Online schema change: dual-write window, backfill via lens, cutover, retire; no write is lost or double-applied | S |
| P2-17 | Row-level security hook consulting the `PolicyEvaluator` trait on every scan; default evaluator allow-all, replaced in Phase 4 | T |
| P2-18 | Query cancellation, statement timeouts and memory limits per query | T |
| P2-19 | Vacuum and compaction of row versions below the oldest snapshot, bounded by retention policy; history in the log is retained | S, B |

## Tests, evals and benchmarks added

- Model-based tests of every table operation against a map oracle, with crash, recover, branch and `AS OF` operations in the sequence.
- Simulation scenarios with two engines, fault profiles from Phase 1, invariants I1, I2, I3, I9.
- Snapshot isolation anomaly tests and transaction-history checks (Elle) on simulated histories.
- Differential tests of the SQL subset against PostgreSQL on generated queries over the same data, comparing results (only for constructs in the subset).
- Benchmarks: point read, range scan, insert, update, mixed workload shaped like an issue tracker and wiki; recovery time; comparison with PostgreSQL as a report, not a gate.
- Fuzzing of the PG wire decoder and the SQL parser.
- North star **NS1** now passes for log and row engines.

## Exit criteria

| # | Criterion |
|---|---|
| E2-1 | I2 for log plus row holds across 1,000,000 simulation seeds per night for seven nights |
| E2-2 | I3 and I9 verified in simulation; scrubbers clean on soak data |
| E2-3 | `sqllogictest` declared subset 100 percent; driver matrix green; differential test with zero result mismatches on the subset |
| E2-4 | Planted bugs extended with at least 10 storage-level bugs, all detected |
| E2-5 | Benchmark baselines for the issue-tracker workload recorded, no unexplained regression against Phase 1 |
| E2-6 | Recovery time and checkpoint behaviour documented with data |
| E2-7 | NS1 passes with log and row engines |
| E2-8 | Planner ADR accepted; SQL subset document frozen for this phase |

## Depends on, risks

Depends on Phase 1. Risks: SQL scope creep (the subset is declared and conformance-driven); MVCC and vacuum bugs (model-based plus simulation); fallback to a Postgres-derived SQL layer if the pivot trigger in the direction document fires.
