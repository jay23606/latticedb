# Roadmap

Where the project is, phase by phase. The sources of truth are the phase pages in [docs/gates](gates),
the [specification](spec/README.md), the [milestones](https://github.com/moncifem/latticedb/milestones)
and the [project board](https://github.com/users/moncifem/projects/1). This page summarises them; if it
disagrees with a gate page, the gate page is right and this page needs a fix.

Status below is what `main` does on 2026-10-30. Items marked *not on main* are planned, not built.

## Status

| Phase | Goal | State | Gate |
|---|---|---|---|
| 0 Repository and conventions | Workspace, CI, governance | Done (gate record dated 2026-10-28) | [phase-0](gates/phase-0.md) |
| 1 Foundations and evaluation | Durable log, simulation, schema language, verification instruments | Built; evidence still accumulating (nightly history, review package). Gate not signed | Open: [phase-1](gates/phase-1.md), [review](gates/phase-1-review.md) |
| 2 Storage engines and SQL | MVCC row engine, SQL over the PostgreSQL wire | In progress. On main: single-table `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `DROP TABLE` with durable column definitions across restart (RFC 0012, accepted 2026-10-30; the seven-day comment hold was first kept, then waived by the owner in chat on 2026-10-30, see #712), SCRAM over TLS, `psql` suite, container image. *Not on main:* joins, aggregates, subqueries, driver matrix (#413), `ALTER TABLE`, branches, checkpoints | Open: [phase-2](gates/phase-2.md) |
| 3 Sync, CRDTs and SDKs | Live subscriptions, offline merge, SDKs, agent plugin contract | Started. On main: CRDT building blocks, change-feed and sync codecs, principal-bound subscriptions, plugin contract v1, MCP codec with a read, write and subscribe backend over the executor, SDK outbox, an experimental WebSocket gateway on loopback (`latticed --gateway-listen`, bearer tokens with reload; no TLS, read only, rows only), experimental TypeScript and Python codec clients. *Not on main:* MCP over HTTP or stdio, `lattice watch`, TLS on the gateway, live queries, collaborative documents | Open: [phase-3](gates/phase-3.md) |
| 4 to 12 | Identity and policy, blobs and S3, schema v1 and packs, search, intelligence, role packs, cluster, optional models, 1.0 | Not started | None yet |

Phase 2 runs ahead of the Phase 1 gate under the waiver in [docs/gates/phase-1-review.md](gates/phase-1-review.md)
(granted 2026-10-28). That waiver covers Phase 2 only: no Phase 2 release tag and no Phase 2 exit-criteria claim
before the Phase 1 gate is signed. Phase 3 started on 2026-10-29 under the same conditions as that waiver,
as [recorded in its gate page](gates/phase-3.md); no Phase 3 release tag and no Phase 3 exit-criteria claim
before the earlier gates are signed.

## What you can do today

Start `latticed --data-dir DIR --development-policy allow-all` (loopback trust) or the SCRAM-over-TLS
mode, connect with `psql` or another PostgreSQL client, and run single-table SQL against a write-ahead log.
Tables and their column definitions come back after a restart. The experimental sync gateway runs when
`--gateway-listen` is given. The `lattice account` tool and the container image are on main. See the
[README](../README.md) and the [alpha guide](alpha.md) for commands and limits.

## What comes next

1. Alpha 0.1 (milestone): the install script and release rehearsal (#585, #591), the quickstart from the
   built archive (#586), `lattice watch` (#583), TLS on the gateway (#578), MCP over HTTP (#527, #580),
   the end-to-end harness and two-agent demo (#540, #534), the security disclosure refresh (#590) and the
   alpha documentation (#589). No tag is cut before the owner decides (#568).
2. SQL: the `kill -9` restart proof with the real binary (#574), the driver matrix and binary formats
   (#413, #576, #607, #608, #710), then joins and aggregates (#380, #592 to #597). The in-house planner
   is decided ([ADR 0025](adr/0025-sql-planner-in-house.md)).
3. Owner decisions still open for the alpha are listed in #568, section D (among them a pre-release tag before the Phase 1 gate).
4. Phase 1 evidence: more nightly passes, mutation score, Linux runtime numbers, two-maintainer sign-off.
5. Draft formats (plugin contract, change feed, sync protocol) go through an RFC, golden files and an
   independent review before they are frozen.

## Work tracks

- Phase milestones hold one issue per testable work package, created when the previous phase nears its
  gate. Phases 4 to 12 therefore show only their phase issue for now.
- The *Engineering platform* milestone holds merge safety, governance and verification tooling.
- The *Alpha 0.1* milestone holds the first test release. Its definition and critical path are in #567,
  its release checklist in #568.

## Keeping this current

Every pull request that changes what works, how to run it or the status updates the README, the gate
page for its phase, the changelog and, when a phase changes state, this page. The owner signs gates; no
page claims a gate passed before that.
