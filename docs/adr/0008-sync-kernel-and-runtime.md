# ADR 0008: Synchronous kernel behind seams; runtime decision deferred

Status: accepted

## Context

Deterministic simulation needs every source of nondeterminism behind a trait and a single-threaded
scheduler. The engine runtime (tokio versus thread-per-core with io_uring) is not decided.

## Decision

The Phase 1 kernel is synchronous and generic over `Clock`, `Rng` and `Io`. Concurrency is a shell
around it (a commit pipeline owned by one thread). The async runtime for the API and service layers
and the engine I/O model remain open and are decided from benchmarks (backlog P1-62, P1-63), not now.

## Consequences

The simulator runs the real kernel code with no executor tricks. A later async I/O layer must
preserve the same durability contract documented on `Io`.
