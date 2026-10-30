# Architecture overview

Lattice is a Rust workspace of crates arranged in layers. Each crate declares the crates it may depend on
in `layers.toml`, and layers depend only downwards. `cargo xtask check-layers` fails the build on a forbidden
dependency. The one declared exception is `lattice-cli` depending on `latticed` for the owner-only file
helpers, tracked in [#686](https://github.com/moncifem/latticedb/issues/686).

Deterministic crates may not use real time, the filesystem, the environment, threads or `HashMap`. The
rules are in [`clippy.toml`](../../clippy.toml), and an exception has to be listed with its reason.

## The layers

| Layer | Crates | What they do |
|---|---|---|
| 0 | `lattice-sys`, `lattice-abi` | OS shims; ids and the wire and ABI types |
| 1 | `lattice-seams`, `lattice-proto` | I/O, clock and random traits (with real and simulated implementations); sync frame and WebSocket codecs |
| 2 | `lattice-lsl`, `lattice-sim`, `lattice-sql` | Schema language (parse, check, compatibility, typed rows); deterministic simulator; SQL parser and binder |
| 3 | `lattice-core` | The kernel: write-ahead log, transactions with fencing and deduplication, checkpoints, recovery, scrubber, CRDT modules |
| 4 | `lattice-engines` | The row engine: MVCC tables, snapshot reads, secondary indexes, catalog, version vacuum |
| 4 | `lattice-identity` | Placeholder for the identity layer (a later phase) |
| 5 | `lattice-exec` | Executor for the single-table SQL subset, with durable table definitions |
| 5 | `lattice-service`, `lattice-packs`, `lattice-intel` | Placeholders for the workspace, schema-pack and intelligence layers (later phases) |
| 6 | `lattice-api` | PostgreSQL wire codec, a sans-I/O session state machine, subscription host and backpressure |
| 6 | `lattice-sdk` | SDK pieces: sync client state machine, outbox, plugin contract, MCP codec and grant layer |
| 7 | `latticed`, `lattice-cli` | The server (listener, authentication, TLS, the experimental gateway and the MCP tool backend) and the `lattice` command line tool |
| 8 | `lattice-testkit`, `lattice-evalkit`, `lattice-bench` | Test harnesses, evidence reports and benchmarks |
| 9 | `xtask` | Repository tooling that CI runs. It is not shipped |

## How a query travels

A PostgreSQL client connects to `latticed`. The listener in `latticed` owns the TCP socket, TLS and
authentication. `lattice-api` decodes the wire messages and runs the session state machine. The statement
goes to `lattice-sql`, which parses, binds and plans it. `lattice-exec` runs the plan against the row engine
in `lattice-engines`, which keeps the rows in MVCC form on top of the kernel in `lattice-core`. The kernel
writes every committed change to the write-ahead log before it acknowledges it. The simulator in
`lattice-sim` replays the same kernel code against a fake disk that can crash, tear writes and inject faults.

The SQL is PostgreSQL's dialect, not a new language. Where a construct is accepted it should return what
PostgreSQL returns, and where it is not supported it is refused with SQLSTATE `0A000` rather than
approximated. The [SQL subset](sql.md) page lists what runs today. The schema language, LSL, is a separate
language with its own parser and compatibility checker. In the current code, `CREATE TABLE` is ordinary SQL.
Deriving DDL from LSL is a plan in the subset document, not a feature that exists yet.

## What is built and what is not

- Built and tested: the kernel, the row engine as a library, single-table SQL over the wire, SCRAM over TLS,
  per-table grants, the LSL tools, CRDT modules and the codecs.
- Experimental: the WebSocket sync gateway and the MCP tool backend. See [Agents and live collaboration](agents.md).
- Not built: identity and policy compilation, encryption at rest, the audit chain, joins and aggregates, and
  most of the later phases. The [threat model](../threat-model.md) lists which boundaries are real today.

## Design documents

[`ARCHITECTURE.md`](../ARCHITECTURE.md) is the design proposal for the whole product. It is a proposal, not a
description of the code: check the layers above before relying on any part of it. The engineering plan is in
[`ENGINEERING.md`](../ENGINEERING.md). Decisions are in the [ADRs](../adr) and the [RFCs](../rfc). Byte formats
live in [`formats/`](../../formats) with golden files. The [glossary](../glossary.md) defines the terms.
