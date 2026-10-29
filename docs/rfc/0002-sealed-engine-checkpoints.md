# RFC 0002: Sealed engine checkpoints and suffix recovery

Status: draft
Comment period ends: 2026-11-05
Decision ADR: none yet

## Motivation

P2-08/#118 and #108 require recovery work to depend on a checkpoint and its suffix rather than
replaying the entire WAL. Existing type-3 checkpoint records check dedupe/fence consistency but
contain no engine state. Streaming full replay avoids collecting records; it still reads every
segment and rebuilds every historical write.

## Proposal

Introduce the bounded, chunked format in `formats/engine-checkpoint-v1.md`. A new opaque WAL type-4
seal authenticates the manifest after all snapshot files and their directory entries are durable.
The source WAL is synced and rotated to a fresh segment before any snapshot publication. One writer
lock and an exclusive mutable kernel borrow serialize snapshot publication with commits. Logged but
unapplied transactions are refused; existing read snapshots and buffered engine transactions are
safe because every retained historical version is preserved.

A checkpoint stores kernel dedupe IDs, turn fences, HLC, and a tagged engine state. Raw RowStore
stores every retained version/tombstone and writer ID, not merely current values. Secondary indexes
are rebuilt from that history using caller-supplied definitions. Legacy CatalogStore preserves all
catalog versions and permanently used table IDs. Main currently has separate row/catalog views;
wrong-engine snapshots trigger full replay. Draft #151's unified typed store needs a new combined
engine codec/tag which includes schema source, compiled-layout history and raw-history IDs; this
proposal does not duplicate or discard that work.

Recovery loads and validates snapshot files while holding the WAL lock, checks the seal as the
first record in its anchor segment, verifies the inventory of skipped segment names and lengths,
restores state into a private candidate, and replays the anchor and suffix with the original global
LSNs. State becomes visible only after successful restore and suffix validation/sync. A missing,
wrong-engine, malformed, unsealed or semantically invalid cache triggers full WAL replay. Device
errors other than missing paths propagate. A failed cache never licenses ignoring WAL corruption:
full recovery must still succeed before returning a store.

Checkpoint artifact generation names conservatively preserve the durably established source LSN.
Full replay cannot initialize an empty replacement WAL or accept a log ending below that boundary
while such evidence remains, even when cache bytes are damaged. A through-zero empty-source orphan
does not claim nonempty history. The checkpoint namespace is reserved: malformed bytes fall back,
but a forged/damaged generation name claiming a later source may force refusal. Erasure of both
all WAL and all checkpoint evidence remains indistinguishable from a fresh database.

The sealed state is authoritative for skipped WAL prefix bytes. Fast opening checks their names
and lengths, not their contents. Prefix bit rot that preserves length requires full scrub to detect;
this is explicit evidence scope, not an integrity claim about unread bytes. No WAL is pruned.

The new `Io::visit_dir` default preserves compatibility by collecting `list`. StdIo and SimIo provide
bounded traversal; transparent wrappers delegate. A backend relying on the default does not receive
the bounded-directory-allocation claim. Tail segment names still consume memory proportional to
the suffix segment count; this is not a constant-memory recovery API.

## Alternatives

* Replay the full WAL with a streaming visitor: already implemented; does not bound historical IO.
* Latest-value snapshot plus pruning: breaks retained snapshots, tombstones, writer provenance,
  catalog time travel and deduplication; outside scope.
* One monolithic snapshot: incompatible with the whole-file `Io::read` seam's allocation behavior.
* Replace type-3 payload: breaks existing readers. A new opaque record type retains compatibility.
* Add range reads and atomic rename: larger seam surface; immutable chunks and a WAL seal suffice.

## Compatibility and migration

The segment header/version and existing record layouts are unchanged. The unchanged legacy kernel
open path is tested to replay original changesets and ignore type-4 seals, then pass full scrub.
That is executable compatibility evidence for this repository's existing parser, not a promise
about unknown third-party implementations. Snapshot files are in a separate `.checkpoint` directory.
Rollback can ignore the directory and replay retained WAL. Unknown snapshot versions/tags are refused
as cache candidates and fall back to WAL; known engine payload corruption in the WAL remains fatal.
There is no destructive migration or garbage collection. Incomplete generations remain orphaned.
The newest invalid generation falls back to full replay rather than searching older generations.

This RFC and proposed ADR0016 remain draft/proposed for at least seven days after the PR opens.
The date above must move later if publication occurs after 2026-10-29. Acceptance and ADR indexing
are separate review steps; passing CI does not waive the comment period.

## Validation and measurement

Tests cover multi-chunk state, all historical row/index/catalog snapshots, dropped ID refusal,
wrong-engine fallback, missing/corrupt cache, missing WAL failure, dedupe/fences/HLC, publication
power cuts at every mutating operation and recovery-sync power loss. Instrumented IO verifies
zero reads of prefix segments, zero collecting directory calls, and snapshot buffers at most
256 KiB for growing retained histories. Decoder tests cover truncation, trailing bytes, oversized
counts/lengths, overflow and arbitrary bytes.

These establish buffer/IO behavior, not a measured process RSS bound. Retained MVCC data, indexes,
catalog versions and dedupe IDs necessarily grow with history. Before P2-08 is called complete,
measure fresh-process peak RSS and elapsed recovery for geometrically increasing history sizes,
with identical datasets, checkpoint positions and suffix sizes. Report full replay and checkpoint
recovery separately, include filesystem/cache conditions, dedupe/history sizes and largest WAL
segment. Use `/usr/bin/time -l` on macOS or `/usr/bin/time -v` on Linux around separate recover-only
processes, not a process that constructed the database. WAL buffers are bounded by the largest
actual segment, which may exceed the soft rotation target after a large group commit. No claim of
constant total resident state or WAL-content verification is made.

Independent byte-exact golden fixtures and a separate generator are still a draft acceptance gap.
The initial corpus tests cover structural encode/decode and malformed bytes, not independent format
interoperability. Neither those fixtures nor the RSS/time table are claimed complete by this draft.
