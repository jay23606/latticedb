# RFC 0003: Checkpoint schemas and typed rows together

Status: draft
Comment period ends: 2026-11-05 (at least seven full days after this RFC's PR opens)
Decision ADR: proposed ADR 0017, not accepted

## Motivation

RFC 0001 makes the catalog and row store share a WAL, with immutable writer layouts. RFC 0002 adds
checkpoint recovery for raw rows and legacy catalog views. A row-only checkpoint cannot restore typed
rows: it lacks the original layouts, schema timeline and raw physical-id reservations. Skipping the
prefix without those facts could reinterpret old values or reuse a deleted raw table id.

This integration stays on a separate draft branch. Neither parent RFC is accepted by combining its
code here. Do not merge this proposal before both parents are accepted and its own seven-day review
and compatibility evidence are complete. Phase 1 remains unsigned; no Phase 2 exit is implied.

## Proposal

Add distinct checkpoint engine tags for a complete typed catalog and for the combined row/catalog
state. Preserve every catalog version and used id, every original checked schema and actual table creation/drop positions (including invisible atomic lifetimes), all retained row
versions and tombstones, and the raw table-id reservation set. Kernel dedupe, turn fencing and HLC
state use RFC 0002 unchanged. Rebuild indexes from the retained rows.

The schema codec stores canonical LCAT v2 DefineSchema bytes plus the schema's original installation
LSN, recompiling checked LSL on restore. Compiler implementation structs are not serialized. Decode
counts, ordering, reserved ids, schema LSNs and canonical semantics before exposing restored state.
Validate each typed row against its writer schema at that row's historical LSN, preserving I8 after
schema evolution. A schema cannot adopt older raw row history. New writes still require the current
schema and the wrapper's existing catalog fence.

Snapshot framing, digest validation, durable publication and source-WAL loss refusal remain those of
RFC 0002. Missing, malformed or wrong-engine snapshots fall back to verified full replay; they never
skip state owned by another engine. Existing raw/legacy tags are not reinterpreted as combined state.

## Alternatives

Accepting a raw-row snapshot and reconstructing schemas from the skipped WAL defeats the prefix-skip
contract and may lose the writer layout. Saving only the newest schema changes historical decoding.
Saving only live rows loses snapshot history and id reservations. These alternatives are unsuitable.

## Compatibility and remaining evidence

See `formats/typed-checkpoint-v1.md`. This adds new engine tags under the draft checkpoint container;
old readers must use retained WAL and cannot assume new typed schemas are writable. No WAL is pruned.
Wrong tags cause full replay. No downgrade converter or automatic migration is supplied.

Review evidence includes fast-path layout/id-reservation tests, corrupt-cache full-replay tests and
an every-operation publication-cut sweep. Independent byte-exact combined-state fixtures, measured
RSS/time for combined typed state and composition with vacuum remain required before acceptance.
