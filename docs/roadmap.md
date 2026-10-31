# Roadmap

Where the project is, phase by phase. The sources of truth are the phase pages in [docs/gates](gates),
the [specification](spec/README.md), the [milestones](https://github.com/moncifem/latticedb/milestones)
and the [project board](https://github.com/users/moncifem/projects/1). This page summarises them; if it
disagrees with a gate page, the gate page is right and this page needs a fix.

Status below is what `main` does at `ffa2f9de` on 2026-10-31. Items marked *not on main* are planned, not built.

## Status

| Phase | Goal | State | Gate |
|---|---|---|---|
| 0 Repository and conventions | Workspace, CI, governance | Done (gate record dated 2026-10-28) | [phase-0](gates/phase-0.md) |
| 1 Foundations and evaluation | Durable log, simulation, schema language, verification instruments | Built; evidence still accumulating (nightly history, review package). Gate not signed | Open: [phase-1](gates/phase-1.md), [review](gates/phase-1-review.md) |
| 2 Storage engines and SQL | MVCC row engine, SQL over the PostgreSQL wire | In progress. On main: single-table `SELECT` (with `ORDER BY`, `LIMIT` and `OFFSET`), `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `DROP TABLE`, index scans for primary key and `UNIQUE` predicates, durable table definitions across restart (RFC 0012 and ADR 0028 accepted; the seven-day comment hold was waived by the owner in chat, see #712), SCRAM over TLS and loopback trust, the `psql` suite plus psycopg, node-postgres and pgx smoke scripts in CI, the container image, the `lattice account` tool. *Not on main:* joins, aggregates, `GROUP BY`, `DISTINCT`, subqueries (#380, #592 to #597), the driver matrix (#413), binary formats (#608), `ALTER TABLE` and online schema change, branches, checkpoints | Open: [phase-2](gates/phase-2.md) |
| 3 Sync, CRDTs and SDKs | Live subscriptions, offline merge, SDKs, agent plugin contract | Started. On main: CRDT building blocks, change-feed and sync codecs, principal-bound subscriptions, plugin contract v1, MCP tools `lattice_read`, `lattice_write` and `lattice_subscribe` over the SQL executor, served over HTTP at `POST /mcp` on the gateway port (bearer token, loopback, no TLS, with the Host, Origin, JSON-only and bounded-body checks of #580), SDK outbox, an experimental WebSocket gateway on loopback (`latticed --gateway-listen`, bearer tokens with reload; no TLS, read only, rows only), experimental TypeScript and Python codec clients. *Not on main:* MCP over stdio, the per-principal long-poll cap for MCP (#529), the remaining #580 acceptance items (its Host, Origin and token checks are on main; the issue stays open until its list is closed), `lattice watch` (#583), TLS on the gateway (#578), live queries, collaborative documents | Open: [phase-3](gates/phase-3.md) |
| 4 to 12 | Identity and policy, blobs and S3, schema v1 and packs, search, intelligence, role packs, cluster, optional models, 1.0 | Not started | None yet |

Phase 2 runs ahead of the Phase 1 gate under the waiver in [docs/gates/phase-1-review.md](gates/phase-1-review.md)
(granted 2026-10-28). That waiver covers Phase 2 only: no Phase 2 release tag and no Phase 2 exit-criteria claim
before the Phase 1 gate is signed. Phase 3 started on 2026-10-29 under the same conditions as that waiver,
as [recorded in its gate page](gates/phase-3.md); no Phase 3 release tag and no Phase 3 exit-criteria claim
before the earlier gates are signed.

## What you can do today

Start `latticed --data-dir DIR --development-policy allow-all` (loopback trust) or the SCRAM-over-TLS
mode, connect with `psql` or another PostgreSQL client, and run single-table SQL against a write-ahead log.
Tables and their column definitions come back after a restart. The `lattice account` tool provisions the
credential files that `latticed` reads: SCRAM verifiers, account policies and gateway bearer tokens.

The experimental sync gateway runs when `--gateway-listen` is given, and serves the MCP tools at `POST /mcp`
on the same port. Both are loopback only and have no TLS. The Docker dev stack in [dev](../dev) runs the published
image with SCRAM over TLS on `127.0.0.1` for maintainers and their agents. A shell helper and schema for a shared
agent task board run over `psql` in [integrations/board](../integrations/board/CONVENTIONS.md). See the
[README](../README.md) and the [alpha guide](alpha.md) for commands and limits.

## What comes next

1. Alpha 0.1 (milestone): `v0.1.0-alpha.1` is tagged. The milestone still holds the install script for the
   release archives (#585), the quickstart from the built archive (#586), the release checklist and rehearsal
   (#591, #568), `lattice watch` (#583), TLS on the gateway (#578), the browser and DNS-rebinding defences
   for MCP over HTTP (#580, part 2 of #527; its Host, Origin and token checks are on main, the rest of its acceptance list is open), the per-principal long-poll cap (#529), the end-to-end harness and two-agent demo (#540, #534), the
   security disclosure refresh (#590) and the alpha documentation (#589). Tags after `v0.1.0-alpha.1` are the
   owner's decision (#568).
2. SQL: the `kill -9` restart proof with the real binary (#574), the driver matrix and binary formats
   (#413, #607, #608, #710), then joins, aggregates, `GROUP BY`, `DISTINCT` and subqueries (#380, #592 to #597).
   The in-house planner is chosen ([ADR 0025](adr/0025-sql-planner-in-house.md)).
3. Alpha 0.2 (dogfooding milestone): the commit-lint scope fix (#736), a CI job for the dev stack (#737) and the
   dev stack follow-ups (#738).
4. Phase 1 evidence: more nightly passes, mutation score, Linux runtime numbers, two-maintainer sign-off.
5. Draft formats (plugin contract, change feed, sync protocol) go through an RFC, golden files and an
   independent review before they are frozen.

## Work tracks

- Phase milestones hold one issue per testable work package, created when the previous phase nears its
  gate. Phases 4 to 12 therefore show only their phase issue for now.
- The *Engineering platform* milestone holds merge safety, governance and verification tooling.
- The *Alpha 0.1* milestone holds the alpha work after the first pre-release. Its definition and critical path
  are in #567, its release checklist in #568.
- The *Alpha 0.2: dogfooding* milestone holds the fixes that the maintainers' own use of the dev stack and the
  agent board turns up.

## Keeping this current

Every pull request that changes what works, how to run it or the status updates the README, the gate
page for its phase, the changelog and, when a phase changes state, this page. The owner signs gates; no
page claims a gate passed before that.
