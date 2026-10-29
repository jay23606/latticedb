# ADR 0015: Persist schemas with typed rows' writer-layout versions

Status: proposed

## Context

P2-01/P2-02 require catalog-owned typed tables and durable layouts. Typed rows previously needed a
caller-supplied layout; replaying old bytes with reordered columns or enum variants could change their
meaning. RFC 0001 proposes the format extension and remains in its required comment period.

## Decision

Proposed, not accepted: persist canonical checked LSL, immutable schema versions and stable per-type
table ids in LCAT v2. Use one RowStore WAL/kernel for catalog and rows. Typed row envelopes reference
the writer-schema version, so restart and historical reads recover the original interpretation.
Reserve physical ids from all row history, including tombstones; validate typed values before append
and fence writing transactions if the catalog changed. Keep legacy v1 catalog encoding unchanged.

Do not merge the format implementation or mark this ADR accepted before the RFC review period ends
and its compatibility and safety findings are resolved.

## Consequences

Old rows remain readable under their writer constraints without requiring caller layouts at reopen.
New schemas do not implicitly migrate old data. The draft catalog/envelope formats, independent
Python golden files, semantic replay refusal tests and power-cut regressions provide review evidence.
The raw-history id-collision found during independent review has a fix and regression coverage.

There is no safe automatic downgrade, online backfill, persisted index configuration or completed
Phase 2 gate. The RFC records the backup/rollback restriction and old row-only client limitation.

Schema validation is provided by `TypedStore`. The public raw `RowStore` byte API remains an escape
hatch and must not write typed tables after adoption; it is not prevented from storing invalid typed
bytes. The RFC records this limitation for review rather than promising engine-wide enforcement.
