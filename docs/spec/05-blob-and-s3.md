# Phase 5: blob engine and S3-compatible API

## Goal

Build our own object store inside the engine: content-addressed, transactional, fast, and reachable by any S3 SDK. References are part of the same commit as the entity change, so there are no orphans or dangling attachments.

## Scope

**In:** chunker, content hashing, segment storage, chunk index, refcounts in the transaction, small-object inlining, garbage collection, S3 API subset, SigV4, STS from OIDC tokens, multipart, versioning, presigned URLs, range reads, per-entity encryption keys, blob scrubbers.

**Out:** erasure coding and replication (Phase 10), lifecycle tiering beyond local disks.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P5-01 | Content-defined chunking (FastCDC-class) with BLAKE3 hashing; inline storage below a size threshold set by benchmark | P, B |
| P5-02 | Append-only segment files written with the `Io` seam (io_uring on Linux per the Phase 1 ADR), checksummed, rotated, indexed by hash | S, T |
| P5-03 | Chunk index maps hash to segment and offset; rebuildable from segments (scrubber) | S, T |
| P5-04 | **I11**: references and refcounts change in the same transaction as the entity change; no orphan chunks, no dangling references under any simulated crash | S, P |
| P5-05 | Garbage collection from engine state with a grace period; GC never removes a chunk reachable from any snapshot or branch | S, P |
| P5-06 | Streaming PUT and GET with bounded memory regardless of object size | T, B |
| P5-07 | S3 subset documented: Put, Get (range), Head, Delete, List v2, multipart, bucket versioning, presigned URLs, conditional requests | C |
| P5-08 | SigV4 authentication and STS credentials issued from OIDC tokens; bucket access follows the same policy filter (**I7**) | C, S |
| P5-09 | Per-entity encryption keys for blob content; destroying a key makes all chunks of that entity unreadable (**I10** for blobs) | S |
| P5-10 | Dedupe across entities, with encryption keys handled so that deduplication cannot leak content across tenants (documented design and test) | I, T |
| P5-11 | Blob scrubber verifies hashes against content and refcounts against references (**I11** online check) | T |
| P5-12 | Large object resilience: upload interruption, resume, partial multipart cleanup | S, T |

## Tests, evals and benchmarks added

- Model-based tests against a hash-map oracle; chunker properties (concatenation equals input, stable boundaries after edits, size bounds).
- Crash simulation across blob, log and row engines with I1, I2, I11.
- Ceph `s3-tests` subset and MinIO `mint` subset for the declared API; SDK matrix (AWS SDKs for several languages, `rclone`).
- Benchmarks: PUT and GET throughput and latency by object size, many small objects, dedupe-heavy workloads, concurrent clients; comparison with MinIO on declared workloads as a report. No performance claim is published without the methodology and hardware profile.
- Fuzzing of S3 request parsing and SigV4.

## Exit criteria

| # | Criterion |
|---|---|
| E5-1 | I11 holds across 1,000,000 seeds per night for seven nights; GC safety property tests pass |
| E5-2 | Declared S3 subset passes the selected conformance suites at 100 percent |
| E5-3 | Key destruction makes entity blobs unreadable in simulation, including replicas and backups of the data directory |
| E5-4 | Benchmark baselines recorded for the declared workloads, with a comparison report and methodology |
| E5-5 | Memory stays bounded for multi-gigabyte objects in tests |

## Depends on, risks

Depends on Phase 2 (row and log engines, transactions) and, for the policy path, Phase 4 (can use the stub until then). Risks: dedupe versus encryption tradeoffs (explicit design, tested); GC bugs (the hardest class; planted bugs and simulation target it); performance claims (published only with method).
