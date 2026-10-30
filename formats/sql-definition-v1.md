# Format: SQL table definition v1

Status: proposed. Written from RFC 0012 (Option A, accepted by the owner on 2026-10-30; the RFC and
ADR 0028 are in pull request 414). The RFC's seven-day comment period runs until 2026-11-06 and has
not been waived, so these bytes are not frozen and no writer produces them yet. Nothing on `main`
calls the codec: the envelope that carries it (LCIX version 3), the WAL capability bit 3 that guards
it and the catalog rules are separate work packages. The codec is `lattice-engines::sqldef`.

A definition records how the typed SQL layer reads the rows of one table. It does not contain the
table name (that comes from the catalog operation) or the table id (the definition block carries it).
Definitions are immutable per table id.

## Layout

All integers are unsigned little endian unless a field says signed. All lengths are checked against
the bytes that remain before anything is allocated. A decoder is strict: it rejects unknown versions
and tags, reserved bits, trailing bytes and every non-canonical spelling, so a successful decode
re-encodes to exactly the input.

### Definition body

The body is at most 65,536 bytes.

| Field | Encoding |
|---|---|
| definition version | u8 = 1 |
| layout rule | u8 = 1: the typed-row codec v1 with the type mapping below, columns in this order |
| column count | u16, 1 through 1024 |
| columns | per column, see below |
| primary key | u16 count (0 through the column count), then that many u16 column positions without repeats |
| primary key name | only when the count is above 0: u32 length (1 through 255) and UTF-8 |
| unique count | u8, 0 through 15 |
| unique constraints | per constraint: u16 count (1 through the column count), u16 positions without repeats, u32 name length (1 through 255) and UTF-8 |

Per column:

| Field | Encoding |
|---|---|
| name | u32 length (1 through 255) and UTF-8 with no byte in `0x41..=0x5a` |
| type tag | u8, see below |
| varchar limit | u32; 0 means none; nonzero only for tag 8; counted in characters (Unicode scalar values) |
| flags | u8: bit 0 NOT NULL, bit 1 has a default, bits 2 through 7 must be zero |
| default | only when bit 1 is set: a default tag u8 and its payload |

The name rule is ASCII only for case: no uppercase ASCII byte is allowed and no Unicode folding is
performed or implied. Other UTF-8 is kept as written. Constraint names are not restricted beyond
length; they are stored so that a later change of the executor's naming rule cannot change the names
that error messages report.

### Type tags

1 boolean, 2 smallint, 3 integer, 4 bigint, 5 double precision, 6 numeric (no modifiers), 7 text,
8 varchar, 9 bytea, 10 uuid, 11 date, 12 timestamptz, 13 jsonb. Anything else is refused. A type that
needs a modifier (`numeric(p, s)`) or a new type gets a new definition version, not a new meaning for
a tag.

### Default tags

| Tag | Payload | Column types |
|---|---|---|
| 1 boolean | u8, 0 or 1 | boolean |
| 2 integer | i64 | smallint, integer, bigint; the value must be within the column's range |
| 3 double | u64 bit pattern | double precision; a NaN must be `0x7ff8000000000000` |
| 4 numeric | u32 length and canonical decimal text | numeric |
| 5 text | u32 length in bytes and UTF-8 | text, varchar (character count within the limit when the limit is nonzero) |
| 6 bytea | u32 length and bytes | bytea |
| 7 uuid | 16 bytes as written (big endian) | uuid |
| 8 date | i32 days since 1970-01-01 | date |
| 9 timestamptz | i64 microseconds since 1970-01-01 00:00:00 UTC | timestamptz |
| 10 jsonb | u32 length and UTF-8 text as written | jsonb |

Canonical decimal text is an optional `-`, digits with no leading zero (a single `0` is fine),
optionally `.` and digits with no trailing zero, and never `-0`. Any other spelling is refused, which
is what makes the encoding canonical. Jsonb text is not canonicalised: the executor compares it
textually, so two spellings of the same JSON are two definitions. Defaults are constants; `now()` and
expressions are not representable and need a new definition version.

### Structural rules

Checked before append and again on replay, without SQL knowledge:

- column names are nonempty and distinct; constraint names are nonempty, at most 255 bytes and
  distinct, the primary key's name included;
- every key position is in range and not repeated within a key; every primary key column is NOT NULL;
- no key column (primary key or unique) is numeric or jsonb;
- a default's tag matches the column type and is in range; reserved flag bits are zero; a varchar
  limit appears only on varchar;
- at most 15 unique constraints. Unique constraint `k` uses the marker table
  `0x8000_0000 | id << 4 | k`, with `k` its position here; the format freezes that mapping. The table
  id must therefore be at most 2^26 - 1.

The executor re-checks the semantics it owns when it turns a definition into a table (for example that
a numeric default parses and a jsonb default is valid JSON).

### Definition block

The unit an envelope carries is a block of definition operations, in ascending table id.

| Field | Encoding |
|---|---|
| count | u32, 1 through 1024 |
| operations | per operation: tag u8 = 1 (DefineTable), table id u32 (at most 2^26 - 1, strictly greater than the previous id), body length u32 (at most 65,536), then the body |

Other tags are refused; the tag space is left for later kinds (views, sequences).

## Versioning and compatibility

The definition version and the layout rule are separate bytes so either can change without the other.
A change of the body is a new definition version; a change of how rows are laid out is a new layout
rule. Readers refuse versions and rules they do not know. Embedding the block in LCIX version 3 and
refusing it in readers that lack WAL capability bit 3 is RFC 0012's envelope work and is not part of
this codec. Layout rule 1 keeps columns in declaration order; a decoder that reorders them would read
every stored row wrongly, which is why the codec has a planted bug for it (`SqlDefColumnsSorted`).

## Checksums

None are embedded. The enclosing ChangeSet and WAL framing provide the CRC.

## Golden files

`formats/generate_sql_definition_golden.py` frames every vector with Python `struct`; no Rust codec is
called. `python3 formats/generate_sql_definition_golden.py --check` verifies the tracked bytes. The
files in `corpus/` carry a `.golden.toml` with status `proposed` until the owner freezes the format.

| Corpus files | Contents |
|---|---|
| `sql-definition-v1-ok-minimal` | One nullable integer column `id`, no keys (19 bytes) |
| `sql-definition-v1-ok-all-types` | One column per type tag 1 through 13, varchar with limit 10 |
| `sql-definition-v1-ok-all-defaults` | Every default tag, a canonical NaN, a multi-byte varchar default at its limit |
| `sql-definition-v1-ok-composite-key` | A two-column primary key and one unique constraint |
| `sql-definition-v1-ok-fifteen-uniques` | The most unique constraints |
| `sql-definition-v1-ok-non-ascii-name` | Columns `é` and `É`: both accepted, since only the bytes `A` through `Z` are banned and nothing is folded |
| `sql-definition-v1-ops-ok-*` | Blocks of two definitions and of the largest table id |
| `sql-definition-v1-reject-*` | One input per body rule: version, layout rule, column count, type and default tags, flags, names, duplicate columns, default mismatches and ranges, non-canonical numeric and NaN, key positions, nullable and numeric keys, constraint names, sixteen uniques, trailing bytes, lengths past the end |
| `sql-definition-v1-ops-reject-*` | One input per block rule: empty, 1025 operations, operation tag, table id over the limit, not ascending, duplicate id, body over 64 KiB, lengths past the end, trailing bytes |

`crates/lattice-engines/tests/sqldef_golden.rs` pairs each file with a literal expected outcome. Every
strict prefix of every accepted body and block is refused, and every single-bit flip of them is refused or re-encodes to the flipped bytes. The size boundaries (1024 and 1025 columns, 255 and 256 byte names, 65,536 and 65,537 byte bodies, 1024 and 1025 operations) are tested in the same file. The fuzz target `sqldef_decode` is seeded by
`fuzz/seeds/generate_sqldef_seeds.py` from the same vectors.
