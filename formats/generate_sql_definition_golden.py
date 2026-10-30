#!/usr/bin/env python3
"""Independent vectors for the SQL table definition format (RFC 0012, formats/sql-definition-v1.md).

Standard library only. Every byte is framed here with `struct`; no Rust codec is called and no
expected value is derived by running the code under test. Run from any directory:

    python3 formats/generate_sql_definition_golden.py          # write formats/corpus/sql-definition-v1-*
    python3 formats/generate_sql_definition_golden.py --check  # fail when the tracked bytes differ

Files named `sql-definition-v1-ok-*` are accepted bodies, `-ops-ok-*` accepted definition blocks and
`-reject-*` / `-ops-reject-*` inputs a conforming decoder must refuse. The Rust test
`crates/lattice-engines/tests/sqldef_golden.rs` pairs each file with its literal expected outcome.
"""
import argparse
import struct
import sys
from pathlib import Path

CORPUS = Path(__file__).resolve().parent / "corpus"
STATUS = "proposed"  # RFC 0012 is accepted by the owner, but its comment period runs to 2026-11-06.
NAN = 0x7FF8000000000000


def u8(n):
    return struct.pack("<B", n)


def u16(n):
    return struct.pack("<H", n)


def u32(n):
    return struct.pack("<I", n)


def sized(data):
    return u32(len(data)) + data


# Default payloads: tag byte then the payload of the format document.
def d_bool(v):
    return u8(1) + u8(v)


def d_int(v):
    return u8(2) + struct.pack("<q", v)


def d_double_bits(bits):
    return u8(3) + struct.pack("<Q", bits)


def d_numeric(text):
    return u8(4) + sized(text.encode())


def d_text(text):
    return u8(5) + sized(text.encode())


def d_bytes(data):
    return u8(6) + sized(data)


def d_uuid(data):
    assert len(data) == 16
    return u8(7) + data


def d_date(days):
    return u8(8) + struct.pack("<i", days)


def d_timestamptz(micros):
    return u8(9) + struct.pack("<q", micros)


def d_jsonb(text):
    return u8(10) + sized(text.encode())


def col(name, ty, limit=0, not_null=False, default=None, flags=None, raw_name=None):
    if flags is None:
        flags = (1 if not_null else 0) | (2 if default is not None else 0)
    name_bytes = raw_name if raw_name is not None else name.encode()
    return sized(name_bytes) + u8(ty) + u32(limit) + u8(flags) + (default or b"")


def key(positions, name):
    return u16(len(positions)) + b"".join(u16(p) for p in positions) + sized(name.encode())


NO_PK = u16(0)


def definition(columns, pk=NO_PK, uniques=(), count=None, version=1, layout=1, tail=b"", ucount=None):
    out = u8(version) + u8(layout) + u16(len(columns) if count is None else count)
    out += b"".join(columns) + pk
    out += u8(len(uniques) if ucount is None else ucount) + b"".join(uniques)
    return out + tail


def define_op(table_id, body, tag=1, length=None):
    return u8(tag) + u32(table_id) + u32(len(body) if length is None else length) + body


def block(ops, count=None):
    return u32(len(ops) if count is None else count) + b"".join(ops)


BOOL, SMALLINT, INTEGER, BIGINT, DOUBLE, NUMERIC, TEXT, VARCHAR, BYTEA, UUID, DATE, TIMESTAMPTZ, JSONB = range(1, 14)

MINIMAL = definition([col("id", INTEGER)])


def composite():
    cols = [col("tenant", UUID, not_null=True), col("id", BIGINT, not_null=True), col("title", TEXT)]
    return definition(cols, key([0, 1], "items_pkey"), [key([2], "items_title_key")])


def all_types():
    cols = [col(f"c{tag}", tag, limit=10 if tag == VARCHAR else 0) for tag in range(1, 14)]
    return definition(cols)


def all_defaults():
    cols = [
        col("a_bool", BOOL, default=d_bool(1)),
        col("b_smallint", SMALLINT, default=d_int(-32768)),
        col("c_integer", INTEGER, default=d_int(2147483647)),
        col("d_bigint", BIGINT, default=d_int(-9223372036854775808)),
        col("e_double", DOUBLE, default=d_double_bits(struct.unpack("<Q", struct.pack("<d", -1.5))[0])),
        col("f_double_nan", DOUBLE, default=d_double_bits(NAN)),
        col("g_numeric", NUMERIC, default=d_numeric("-12.034")),
        col("h_text", TEXT, default=d_text("héllo")),
        col("i_varchar", VARCHAR, limit=5, default=d_text("ééééé")),
        col("j_bytea", BYTEA, default=d_bytes(b"\x00\xff")),
        col("k_uuid", UUID, default=d_uuid(bytes(range(16)))),
        col("l_date", DATE, default=d_date(-1)),
        col("m_timestamptz", TIMESTAMPTZ, default=d_timestamptz(1700000000000000)),
        col("n_jsonb", JSONB, default=d_jsonb('{"b": 1,  "a": [true]}')),
    ]
    return definition(cols)


def fifteen_uniques():
    cols = [col(f"c{i}", INTEGER) for i in range(15)]
    return definition(cols, uniques=[key([i], f"u{i}") for i in range(15)])


def sixteen_uniques():
    cols = [col(f"c{i}", INTEGER) for i in range(16)]
    return definition(cols, uniques=[key([i], f"u{i}") for i in range(16)])


def non_ascii_names():
    # RFC 0012 bans only the bytes A-Z and performs no Unicode folding: both spellings are accepted.
    return definition([col("\u00e9", INTEGER), col("\u00c9", INTEGER)])


def ok_bodies():
    return {
        "non-ascii-name": non_ascii_names(),
        "minimal": MINIMAL,
        "all-types": all_types(),
        "all-defaults": all_defaults(),
        "composite-key": composite(),
        "fifteen-uniques": fifteen_uniques(),
    }


def ok_blocks():
    return {
        "two": block([define_op(5, MINIMAL), define_op(9, composite())]),
        "max-table-id": block([define_op((1 << 26) - 1, MINIMAL)]),
    }


def reject_bodies():
    k0 = key([0], "pk")
    nn = col("id", INTEGER, not_null=True)
    r = {
        "version": definition([col("id", INTEGER)], version=2),
        "layout-rule": definition([col("id", INTEGER)], layout=2),
        "zero-columns": definition([], count=0),
        "type-tag-zero": definition([col("id", 0)]),
        "type-tag-14": definition([col("id", 14)]),
        "flag-bit-2": definition([col("id", INTEGER, flags=4)]),
        "flag-bit-7": definition([col("id", INTEGER, flags=0x80)]),
        "uppercase-name": definition([col("Id", INTEGER)]),
        "empty-name": definition([col("", INTEGER)]),
        "name-256-bytes": definition([col("a" * 256, INTEGER)]),
        "name-invalid-utf8": definition([col("x", INTEGER, raw_name=b"\xff")]),
        "duplicate-column": definition([col("a", INTEGER), col("a", TEXT)]),
        "varchar-limit-on-text": definition([col("a", TEXT, limit=5)]),
        "default-tag-11": definition([col("a", INTEGER, default=u8(11) + b"\x00")]),
        "default-mismatch": definition([col("a", INTEGER, default=d_bool(1))]),
        "default-smallint-range": definition([col("a", SMALLINT, default=d_int(32768))]),
        "default-integer-range": definition([col("a", INTEGER, default=d_int(2147483648))]),
        "default-numeric-trailing-zero": definition([col("a", NUMERIC, default=d_numeric("1.50"))]),
        "default-numeric-leading-zero": definition([col("a", NUMERIC, default=d_numeric("01"))]),
        "default-numeric-negative-zero": definition([col("a", NUMERIC, default=d_numeric("-0"))]),
        "default-numeric-text": definition([col("a", NUMERIC, default=d_numeric("NaN"))]),
        "default-nan-noncanonical": definition([col("a", DOUBLE, default=d_double_bits(NAN | 1))]),
        "default-bool-byte-2": definition([col("a", BOOL, default=d_bool(2))]),
        "default-varchar-too-long": definition([col("a", VARCHAR, limit=2, default=d_text("ééé"))]),
        "default-text-invalid-utf8": definition([col("a", TEXT, default=u8(5) + sized(b"\xc3\x28"))]),
        "pk-position-out-of-range": definition([nn], key([1], "pk")),
        "pk-position-repeated": definition([nn, col("b", INTEGER, not_null=True)], key([0, 0], "pk")),
        "pk-count-above-columns": definition([nn], key([0, 0], "pk")),
        "pk-nullable": definition([col("id", INTEGER)], k0),
        "pk-numeric": definition([col("id", NUMERIC, not_null=True)], k0),
        "unique-jsonb": definition([col("id", JSONB)], uniques=[k0]),
        "unique-empty": definition([col("id", INTEGER)], uniques=[u16(0) + sized(b"u")]),
        "pk-empty-name": definition([nn], key([0], "")),
        "constraint-name-256-bytes": definition([nn], key([0], "p" * 256)),
        "duplicate-constraint-name": definition([nn], k0, [key([0], "pk")]),
        "sixteen-uniques": sixteen_uniques(),
        "trailing-byte": MINIMAL + b"\x00",
        "unique-count-short": definition([col("id", INTEGER)], ucount=1),
    }
    # Declared lengths larger than the bytes that follow must be refused before anything is taken.
    r["name-length-past-end"] = u8(1) + u8(1) + u16(1) + u32(200) + b"abc"
    r["default-length-past-end"] = (
        u8(1) + u8(1) + u16(1) + sized(b"a") + u8(TEXT) + u32(0) + u8(2) + u8(5) + u32(0xFFFFFFFF) + b"x"
    )
    r["column-count-past-end"] = u8(1) + u8(1) + u16(1024) + col("a", INTEGER)
    return r


def reject_blocks():
    one = define_op(1, MINIMAL)
    return {
        "empty": block([], count=0),
        "count-1025": u32(1025),
        "op-tag-2": block([define_op(1, MINIMAL, tag=2)]),
        "table-id-over-limit": block([define_op(1 << 26, MINIMAL)]),
        "not-ascending": block([define_op(9, MINIMAL), define_op(5, MINIMAL)]),
        "duplicate-id": block([define_op(5, MINIMAL), define_op(5, MINIMAL)]),
        "body-over-64kib": u32(1) + u8(1) + u32(1) + u32(65537),
        "body-length-past-end": block([define_op(1, MINIMAL, length=len(MINIMAL) + 1)]),
        "trailing-byte": block([one]) + b"\x00",
        "count-past-end": block([one], count=2),
    }


def files():
    out = {}
    for name, data in ok_bodies().items():
        out[f"sql-definition-v1-ok-{name}"] = (data, f"accepted definition body: {name}")
    for name, data in reject_bodies().items():
        out[f"sql-definition-v1-reject-{name}"] = (data, f"definition body a decoder must refuse: {name}")
    for name, data in ok_blocks().items():
        out[f"sql-definition-v1-ops-ok-{name}"] = (data, f"accepted definition block: {name}")
    for name, data in reject_blocks().items():
        out[f"sql-definition-v1-ops-reject-{name}"] = (data, f"definition block a decoder must refuse: {name}")
    return out


def meta(description):
    return (
        'format = "sql-definition"\n'
        "version = 1\n"
        f'status = "{STATUS}"\n'
        f'description = "Independent Python framing, RFC 0012; {description}"\n'
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="verify tracked files without writing")
    args = parser.parse_args()
    bad = []
    for stem, (data, description) in files().items():
        for suffix, content in ((".golden", data), (".golden.toml", meta(description).encode())):
            path = CORPUS / (stem + suffix)
            if args.check:
                if not path.exists() or path.read_bytes() != content:
                    bad.append(str(path))
            else:
                path.write_bytes(content)
    if bad:
        print("stale or missing:", *bad, sep="\n  ")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
