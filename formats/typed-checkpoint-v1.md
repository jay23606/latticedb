# Format: combined typed checkpoint state v1

Status: draft under RFC 0003 and proposed ADR 0017. Parent RFCs 0001/0002 are not accepted by this file.

The container, kernel-state prefix, lengths, chunk/digest checks, source LSN boundary and durable seal
are specified in `engine-checkpoint-v1.md`. This document adds engine-specific tags; all numbers are
unsigned little endian and byte fields use the container's bounded u64-length framing.

## Typed catalog: `typed-catalog-history-v1`

First encode the legacy catalog state: used-id count, each u64 physical id; catalog-version count;
then each version's u64 LSN, table count, and ordered (u64 id, name bytes) entries. This retains dropped
ids and catalog snapshots. Next encode a lifetime count equal to the used-id count, followed by
strictly increasing (u64 id, u64 creation LSN, u64 drop LSN) entries. Zero drop LSN means still live;
creation is positive. Each endpoint must be an actual catalog change LSN within the source boundary.
The lifetime must match every visible catalog version. Create/drop in one atomic change set has equal
endpoints and no visible version; its actual LSN is retained rather than guessed from the used set.
Then encode the schema count. For each immutable schema in increasing
version order, encode its u64 installation LSN and one bounded byte field containing exactly one
canonical LCAT v2 DefineSchema operation with its version, checked LSL source and stable id mapping.

Restore validates legacy ordering/used-id rules, strictly increasing schema versions, nondecreasing
positive installation LSNs present in the catalog timeline and no greater than the source LSN, and
schema ids reserved in the used set. Each schema installation must lie in that id's encoded lifetime;
installation at the same LSN as the drop is valid, later installation is not. The normal catalog semantic operation recompiles each definition;
invalid or noncanonical LSL and incomplete/duplicate mappings are refused. Names may have been renamed
or dropped later; original schema layouts remain immutable.

## Combined engine: `typed-row-catalog-history-v1`

After the kernel prefix, encode:

1. The complete typed catalog state above.
2. Raw physical table-id reservation count, then strictly increasing u64 ids fitting u32.
3. The raw row-history state defined in `engine-checkpoint-v1.md`, including applied LSN, row keys,
   each version's LSN and ChangeSet id, and live-value/tombstone tag.

Every restored row table must be reserved. A typed table cannot have row history at or before its
first schema installation. Each stored typed envelope must identify the schema current at that row's
LSN and satisfy that original schema's constraints; later schema changes do not reinterpret it. Known
dropped typed ids cannot receive raw writes. Index histories are rebuilt from all retained rows.

The old `raw-row-history-v1` and `legacy-catalog-history-v1` container tags are not reinterpreted. New
engines fall back to complete WAL replay when those tags lack their required combined state. No bytes
are skipped under an insufficient tag. Existing log records remain retained; no downgrade is promised.

## Evidence still needed

Independent combined-state fixtures now come from `generate_combined_checkpoint_golden.py`, which
owns LCATv2/LTYP/cells, WAL CRC/framing, lifetime/state and container encoding; only BLAKE3 hashing
uses an external pinned library. The 21 binary files under `corpus/combined-checkpoint-v1/` cover
evolved writer layouts, same-key history, a raw tombstone table-id reservation and an invisible
atomic schema/create-drop lifetime. Tests require actual fast restoration and reproduce a second
complete chunk/manifest/seal through the public stores using explicit test Clock/Rng seams; full
WAL replay cannot masquerade as the fast-path evidence. Existing legacy fixtures remain unchanged.

Fresh-process RSS/time measurements for combined typed state remain pending.
Raw checkpoint measurements do not prove the cost of compiling retained schema histories. Validation
holds at the current tag version; adding fields or relaxing canonicality requires another version.
