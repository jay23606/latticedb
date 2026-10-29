# RFC 0009: Change feed event and cursor encoding

Status: draft
Comment period ends: 2026-11-05
Decision ADR: none yet (proposed ADR 0023)

## Motivation

P3-01 needs an ordered address for each entity change of a committed change set. PR #263 added the
public pure model and codecs before the required RFC/ADR package existed. This proposal records that
review gap; implementation on main is not format acceptance and does not retroactively satisfy review.
No owner acceptance or comment-period waiver is assumed. The seven-day comment period starts with
this proposal on 2026-10-29. ADR 0023 remains proposed until a decision is recorded.

## Proposal

Review the existing candidate bytes in [change-feed-v1](../../formats/change-feed-v1.md) without changing
them. A cursor is exactly 12 bytes: big-endian `lsn u64`, then big-endian `index u32`. Its unsigned byte
order equals `(lsn, index)` order. `(0, 0)` is the only origin representation; no event uses LSN zero.
Other cursor indices are not checked against a change set until an event supplies its size.

An event is version `u8 = 1`, cursor, `set_len u32` little endian, change-set ID (16 big-endian ULID
bytes), HLC (`wall_ms u64`, `counter u32`, `node u32`, all little endian), author (16 big-endian bytes),
turn flag `u8` (zero absent or one followed by little-endian session/turn `u64`s), entity kind `u32`
little endian, key length `u16` little endian and key bytes, then operation `u8`. Operation zero is a
delete; one is an upsert followed by little-endian value length `u32` and value bytes. There is no
checksum, authentication, transport framing, or namespace authorization in this model.

Require `index < set_len` (thus a nonempty event set), keys at most 4096 bytes, and upsert values at most
16777216 bytes. Reject unknown version/turn/operation tags, truncated fields and trailing bytes. Check
lengths against remaining bytes before copying them. The maximum event is 16781405 bytes, including a
present turn; the minimum delete event is 73 bytes. Decoding allocates only bounded key/value buffers.
`events_for` accepts an already allocated caller-supplied vector, with at most `u32::MAX` changes;
this is not a practical batch-memory admission limit. It returns an error without exposing partial
output when a change is invalid. A future host must bound aggregate batch and subscriber memory.

`events_for` copies provenance from its supplied ChangeSet and numbers the supplied entity list. It
cannot establish durability, verify that this list matches the opaque payload, or prevent a caller
from supplying the wrong LSN/order. Integration must derive both from one committed record, use a
deterministic payload traversal, and publish only after durability. Set length describes the complete
unfiltered entity list; any future filtering must preserve original indices and separately specify
transaction boundaries and resume behavior. No filter or atomic subscription delivery exists here.

### Evidence and remaining work

Existing handwritten literal vectors cover an upsert and delete with a turn, plus cursor examples.
Unit/property tests cover codec round trips, malformed fields, bounded decoding, canonical re-encoding,
byte ordering and synthetic increasing LSNs. These establish model properties, not actual durable
recovery, checkpoint replay or subscription atomicity. The `feed_decode` fuzz target and 13 seeds exist,
but the target is not in nightly/weekly/24-hour matrices and execution was not established by PR #263.
No frozen golden corpus exists. Before freezing, independently compare the vectors against this layout,
add reviewed golden fixtures and obtain fuzz execution evidence; runtime activation needs independent
restart/replay and resume/filter tests. No simulator/planted-bug or phase-gate coverage is claimed.

## Alternatives

An LSN alone cannot identify an entity within one compound change set. A timestamp/ID cursor does not
in itself reproduce durable record order. An opaque server token could hide representation but would
still need stable persistent ordering and a separate encoding contract. Keeping pure types without a
public binary codec would avoid committing to these bytes while ordering semantics are reviewed.

## Compatibility and migration

The candidate codecs are public on main but have no production storage, subscription, or transport
caller in the reviewed implementation. They do not change WAL/change-set bytes or checkpoint recovery.
Treat their bytes as experimental: do not persist or promise cross-version compatibility before
acceptance and golden fixtures. If existing experimental consumers persist bytes, inventory them and
agree an explicit versioned migration before revising this candidate; no automatic rewrite is provided.

Accepted v1 bytes must never silently change meaning. Readers currently refuse every version except
one. Any future enclosing wire/storage protocol must independently frame and admit the candidate
format, bound message sizes, and reject unsupported versions before effects; this codec alone supplies
no admission negotiation. A transport checksum/authentication policy cannot be inferred from WAL CRCs.

Checkpoint and vacuum do not renumber retained record LSNs. Future log truncation must preserve those
LSNs and explicitly reject a resume cursor whose history is unavailable; value comparison alone does
not guarantee delivery. Stream identity across unrelated databases, restored histories or generations
also needs a subscription-level contract; the 12-byte cursor has no database/epoch discriminator.
These activation and migration decisions remain outside this pure model proposal.
