# ADR 0017: Restore schemas and row histories as one checkpoint state

Status: proposed

## Context

The catalog/typed-store and raw checkpoint proposals restore different state. A combined engine may
skip a WAL prefix only when its checkpoint restores every original layout and physical-id reservation
alongside the row history. RFC 0003 reviews that composition; RFCs 0001 and 0002 remain separate.

## Decision

Proposed: use distinct typed-catalog and combined typed-row/catalog checkpoint tags. Retain original
checked schema definitions and installation LSNs, full catalog versions and actual table birth/drop LSNs, all row versions/tombstones,
and every raw table-id reservation. Validate typed rows against the schema at each row's historical
LSN; require current schemas for new writes. Use the existing kernel state/publication protocol and
rebuild indexes from rows. Do not reinterpret the raw or legacy tags as complete typed state.

Keep this decision proposed until all parent reviews and RFC 0003's full comment period are complete,
and independent fixtures and measured recovery evidence have been reviewed.

## Consequences

Fast recovery can preserve original-layout reads and reject raw-history adoption after deletion.
Retained state still grows; this is not vacuum, a fixed total memory bound, or a completed phase gate.
New tags require explicit compatibility evidence, while wrong-tag caches safely trigger full replay.
