# Phase 3: live sync, CRDTs, SDKs and the agent plugin contract

## Goal

Make the database live. Clients subscribe to changes and receive them as they happen, work offline and merge, and agents plug in through a contract with fenced turns and resumable streams.

## Scope

**In:** sync gateway (WebSocket, QUIC if justified), subscriptions with resumable cursors, change feed, live queries, offline outbox, CRDT types in the engine (text, tree with moves, set, register), turns and fencing, Rust SDK and generated TypeScript, Go and Python clients, agent plugin contract v1, MCP server v0.

**Out:** identity (tokens are stubbed), search, work queue.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P3-01 | Change feed emits typed events per ChangeSet with a stable cursor; delivery is at least once, ordered per entity and causally across entities | S |
| P3-02 | Subscriptions take a query scope and a cursor; a reconnect with a cursor resumes without gaps or duplicates visible to the client | S |
| P3-03 | Live queries are incremental views over the feed for a defined query class; results equal re-running the query at the same snapshot | P, S |
| P3-04 | Offline outbox in the SDK: queue, retry with idempotent ids, merge on reconnect, clear on ack | S |
| P3-05 | **I5 convergence** for text, tree, set and register under reorder, duplication and delay | S, P |
| P3-06 | Tree CRDT with move semantics and stable entity identity across moves (cycle prevention tested) | P, S |
| P3-07 | Text CRDT integrated with the row engine; history compaction preserving revert points | S, B |
| P3-08 | **I12 fencing** end to end: a superseded turn gets `fenced` and writes nothing, including late writes after reconnect | S |
| P3-09 | Workflow state changes are server-arbitrated, not CRDT; concurrent conflicting transitions create `Conflict` entities | S |
| P3-10 | Agent plugin contract v1 specified in `formats/plugin-contract-v1.md`: session identity by source and external id, idempotent create, append with receipts (`recorded`, `duplicate`, `fenced`), watch stream (`ready`, items, `live`, incremental), turn events, child sessions | I |
| P3-11 | Reference plugin implementation and contract test suite any third-party plugin can run | T |
| P3-12 | Rust SDK handwritten; TypeScript, Go and Python clients generated from LSL and the API description; all pass the same contract tests | T |
| P3-13 | MCP server v0 exposing read, write and subscribe tools scoped to the caller | T |
| P3-14 | Backpressure: slow subscribers are bounded, can be disconnected with a resume cursor, never stall writers | S |

## Tests, evals and benchmarks added

- Convergence simulation across all fault profiles with N replicas; Jepsen-style checker for convergence and no lost acknowledged writes.
- Offline merge scenarios, including both replicas changing workflow state.
- Fencing race scenarios (takeover during append, takeover during watch reconnect).
- Contract test suite for plugins and SDKs; compatibility test between SDK versions and server versions.
- Benchmarks: fan-out latency to 1, 100 and 10,000 subscribers; merge cost; reconnect storm.
- Fuzzing of the sync protocol decoder and plugin JSONL parser.
- North star **NS7** passes.

## Exit criteria

| # | Criterion |
|---|---|
| E3-1 | I5 and I12 hold across 1,000,000 seeds per night for seven nights |
| E3-2 | Contract suite green for the reference plugin and all four SDKs |
| E3-3 | Reconnect with cursor yields zero gaps and zero duplicates across the simulation budget |
| E3-4 | Fan-out and merge benchmarks recorded; no stall of writers under slow-subscriber scenarios |
| E3-5 | NS7 passes end to end through the SDK |
| E3-6 | Text CRDT ADR from Phase 1 confirmed or reversed with data |

## Depends on, risks

Depends on Phase 2. Risks: CRDT history growth (compaction tested early); protocol churn (contract versioned, freeze corpus extended); fan-out cost (benchmarks guide limits).
