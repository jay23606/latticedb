# ADR 0016: Sealed engine checkpoints

Status: proposed

## Context

The kernel's existing consistency checkpoints do not contain engine state. P2-08 and #108 need a
substantive recovery path that can skip a retained WAL prefix without losing historical versions,
writer identities, catalog history, fences, deduplication or HLC monotonicity.

## Decision

Proposed by RFC0002; not accepted. Publish immutable bounded state chunks and a manifest, then sync
a new opaque WAL type-4 seal in a fresh segment. Validate and privately restore the tagged engine
state under the WAL lock, then replay the suffix. Keep all WAL segments for rollback and full scrub.
Treat missing or invalid caches as full-replay fallbacks and propagate device failures.

## Consequences

Fast recovery avoids reading prefix bytes; full scrub remains responsible for auditing their contents.
Every retained version and dedupe ID still consumes memory. Snapshot chunks cap individual snapshot
read buffers, with explicit state size/count limits and fail-stop validation. Separate legacy engine
views use distinct tags; typed store integration needs a combined codec preserving its schema history.
The seven-day RFC comment period and review must finish before acceptance or merging this format.
