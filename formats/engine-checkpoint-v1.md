# Format: sealed engine checkpoint v1

Status: draft, RFC0002; proposed ADR0016. No acceptance or seven-day review waiver is implied.
All integers below are unsigned little endian. Files are immutable after publication.
The existing WAL segment header/version and type-3 consistency checkpoint payload stay unchanged.

## Files and bounds

Files live below `<wal>/.checkpoint/`. A generation basename is
`<seal-lsn:020>-<generation-id:32 lowercase hex>`, exactly 53 ASCII bytes. Readers require lowercase hex; uppercase generations are ignored as
noncanonical cache candidates (no normalization), including by source-evidence traversal. The generation ID is a
fresh ChangeSetId, used only for naming, never installed in the dedupe set. Chunk filenames add
`.<index:04>.chunk`, starting at index zero. The manifest adds `.manifest`.

Snapshot bytes are streamed into 256 KiB chunks. Every chunk except the final one is full, the final
one has 1–262144 bytes, and at most 4096 chunks are permitted (one GiB total state). Over-limit writes
fail; they never silently truncate. Manifest bytes are limited to 256 KiB. Snapshot byte fields
use `u64 length + bytes`, length at most the existing WAL payload cap (16 MiB) and no more than bytes
remaining. Counts are checked against remaining bytes and minimum item sizes before decoding items.
No chunk is read until its filesystem length equals the bounded manifest length; its returned
length and BLAKE3 digest are then checked. Arithmetic and source LSN addition are checked.

## Manifest

| Field | Encoding |
| --- | --- |
| magic/version | 8 bytes `LATCP001` |
| engine tag | length-prefixed bytes, 1–64 bytes |
| through LSN | u64; must permit checked addition of one |
| prefix segment count | u64 |
| prefix total bytes | u64 |
| prefix inventory digest | 32 bytes |
| total state bytes | u64, exact checked sum of chunk lengths |
| chunk count | u64, 1–4096 |
| each chunk | u64 byte length, then 32-byte BLAKE3 of exact file bytes |

Trailing bytes, invalid version/tag bounds, zero/oversized/nonfinal-short chunks, impossible counts,
length sums, or mismatched hashes reject the cache. A future unknown version is not interpreted.

The prefix inventory includes every numeric `.seg` filename recognized by the WAL scanner whose first
LSN is below the seal LSN; writers produce canonical 20-digit names.
For each file, hash `UTF8 basename || u64 filesystem length` with BLAKE3. XOR the 32-byte hashes
(order-independent because directory iteration is unordered), and retain count and total length.
Filesystem names are unique; the writer seals the inventory with the manifest. This detects missing,
renamed, extra numeric or length-changed prefix segments. It does **not** detect same-length byte
changes. The snapshot is authoritative for skipped prefix state; full WAL scrub remains available.
Nonnumeric filenames are ignored by both the inventory and existing WAL scanner.

## WAL seal (opaque record type 4)

| Field | Encoding |
| --- | --- |
| magic/version | 8 bytes `LATCP001` |
| generation basename | length-prefixed 53-byte ASCII basename |
| manifest digest | 32-byte BLAKE3 of exact manifest file bytes |

The seal's record LSN equals `through LSN + 1`. It must be the first record in the matching
`<seal-lsn:020>.seg`, at offset 16 immediately after the standard segment header. The normal WAL
record CRC and strict global suffix LSN order still apply. The manifest filename's LSN must match.
A seal alone without the exact matching manifest and all chunks is not a usable checkpoint.

Older kernel readers preserve type 4 as opaque and replay retained WAL. The unchanged legacy
`Kernel::open` path is tested against a sealed database and full scrub; no claim is made for other
implementations. Unknown checkpoint engine tags trigger full replay, so one legacy engine cannot
skip another engine's history.

## State stream

The complete chunk stream begins with kernel state:

| Field | Encoding |
| --- | --- |
| highest HLC | existing 16-byte HLC encoding |
| committed ID count | u64, at most through LSN |
| committed IDs | each 16 bytes, strictly increasing canonical ID order, no duplicates |
| turn count | u64, at most through LSN |
| turns | each u64 session + u64 current turn; session strictly increasing, turn nonzero |

All ChangeSet IDs are retained. The engine-specific state follows immediately, and must consume
all remaining bytes; no trailing extension is accepted under an existing tag.

### `raw-row-history-v1`

| Field | Encoding |
| --- | --- |
| last applied row LSN | u64, at most through LSN |
| retained version count | u64, minimum 48 bytes per version |
| each version | u64 table ID (must fit u32), length-prefixed key, u64 LSN, 16-byte ChangeSet ID, u64 value tag |
| value if tag 1 | length-prefixed raw row bytes |

Value tag 0 is a tombstone; 1 is a value; other tags are rejected. Versions are strictly ordered by
`(table ID, key bytes, LSN)`. LSNs are nonzero, no greater than last applied LSN; writer IDs must be in
the restored kernel dedupe set. Every version/tombstone is retained, including historical versions
of deleted keys. Empty changesets may advance last applied LSN without adding a version. Index
definitions remain caller supplied; their complete historical entries are derived from ordered
per-row transitions before suffix writes are replayed.

### `legacy-catalog-history-v1`

| Field | Encoding |
| --- | --- |
| used ID count | u64 |
| used IDs | each u64 fitting u32, strictly increasing |
| version count | u64, at most through LSN |
| each version | u64 LSN, then u64 table count |
| each table | u64 ID fitting u32, then length-prefixed UTF-8 name |

Version LSNs strictly increase, are nonzero and at most through LSN. Table IDs strictly increase in
each version, belong to used IDs, and cannot reappear after disappearing from a visible version.
Names are unique within a version and contain 1–255 UTF-8 bytes. IDs created and dropped inside one
atomic changeset can occur only in the used set. All catalog versions and permanently used IDs
survive. This tag deliberately does not encode draft151's typed schema/layout history. Future
integration must introduce a combined tagged codec preserving that additional state and raw-history
ID reservations; it must never reinterpret this legacy tag as a typed snapshot.

## Crash publication protocol

1. Hold the existing WAL writer lock. Reject logged/unapplied transactions. The mutable writer borrow
   serializes publication with commits/checkpoints. Buffered engine transactions remain valid because
   history is preserved; they have not published WAL records.
2. Sync all source WAL bytes. Rotate to an empty segment at the next LSN (or use an already empty
   segment created by ordinary rotation). Sync its header and WAL directory. Capture source state
   through the immediately preceding LSN.
3. Create the checkpoint directory and sync its parent. Write bounded immutable chunks; sync each.
   Compute the skipped-prefix inventory. Sync checkpoint directory entries before writing the manifest.
4. Write/sync the manifest, then sync its directory. A partially published generation is an orphan.
5. Append the matching seal as the anchor segment's first record; sync the WAL before acknowledging.

No WAL files are removed. No manifest or chunk is overwritten. Generation collisions fail. There
is currently no orphan cleanup or cache garbage collection. Publication errors stale the engine
store until reopen; a successful source commit remains recoverable regardless of snapshot failure.

## Recovery and evidence scope

The WAL lock is acquired before reading a candidate snapshot. Inspect checkpoint filenames with
streaming directory traversal and choose the lexicographically newest generation. Validate its
bounded manifest, matching seal, prefix inventory and every chunk; privately decode kernel and
engine state; then scan the anchor and later WAL segments, starting at the seal LSN. The normal WAL
recovery rules apply to the suffix, including torn-tail repair and durability sync. Publish the
opened engine only after the entire operation succeeds. Source WAL below the anchor is not decoded.

Missing/invalid/unsealed/wrong-engine candidates fall back to full WAL replay. Semantically invalid
snapshot state also falls back after discarding the private candidate. Non-missing device errors
propagate; power loss during validation/suffix sync cannot return an opened engine. Checkpoint generation names also preserve a conservative source-LSN floor: publication establishes
that source durably before any chunk is created. A well-formed manifest/chunk generation with seal
LSN greater than one prevents full replay from initializing a replacement empty WAL or accepting a
WAL that ends below that source boundary. Damaged cache bytes do not erase this evidence; a through-zero
orphan does not assert nonempty source history. This guard applies to legacy full opening too. The
checkpoint namespace is reserved: a damaged/forged generation name that asserts a higher boundary
can force refusal rather than fallback availability. Removing all WAL and all checkpoint evidence is
indistinguishable from a new database and is outside this evidence guarantee. Full replay is
still fail-stop on WAL corruption according to the existing WAL rules. The newest invalid candidate
does not trigger an older-candidate search; this can cost performance but cannot lose state.

`CheckpointRecovery` reports through LSN, skipped WAL bytes and state bytes; `RecoveryReport` counts
only suffix records/segments on the fast path. Full replay reports zero checkpoint evidence. State
chunks are read once for validation and once for decode, so `state_bytes` is logical state size,
not total filesystem bytes read. Snapshot read buffers are at most 256 KiB; ordinary WAL reads still
use one whole segment at a time. The normal rotation limit is soft; a large group commit can produce
a larger segment. Tail segment names are collected; their allocation depends on suffix segment
count. Retained state, dedupe sets, indexes and catalog versions grow with history. These limits
must not be described as constant total memory, bounded RSS, or verification of unread prefix bytes.

`Io::visit_dir` has a collecting compatibility default. StdIo/SimIo override it with one-entry-at-a-time
traversal; wrappers must delegate to retain this behavior. A custom backend using the default forfeits
the bounded directory traversal claim.

## Remaining draft validation

`generate_checkpoint_golden.py` independently encodes WAL CRC/framing, kernel and engine history,
chunks, manifests and seals. Its only external primitive is the standard BLAKE3 hash library
[`blake3==1.0.11`](https://pypi.org/project/blake3/1.0.11/); it never calls production checkpoint code.
The 21 binary fixtures under `corpus/checkpoint-v1/` cover empty, row history with tombstones/binary
keys, catalog rename/drop/history and a split chunk boundary. Metadata accompanies each binary.
`--check` reproduces every byte. Rust tests decode/reencode manifests and engine states, reproduce
chunks/seals with the production writer, and compare fast restoration with legacy replay/full scrub.

Fresh-process RSS and timing data are being collected by the opt-in host measurement harness in
`crates/lattice-bench/CHECKPOINT_RECOVERY.md`. Actual CI results remain pending; maximum read-request
sizes alone do not prove allocator peak or resident memory. Format acceptance and the seven-day
comment period remain separate from these tests and measurements.
