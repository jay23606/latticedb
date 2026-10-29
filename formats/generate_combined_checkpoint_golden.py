#!/usr/bin/env python3
"""Independent RFC0003 encoder: owns LCATv2/LTYP/cells, ChangeSet/WAL CRC and container bytes.

Never invokes Rust. Only standard hashing is external: pip install --only-binary=:all: blake3==1.0.11
Run this file to regenerate, or pass --check for byte-exact verification without changing the corpus.
"""
import argparse
from pathlib import Path
import struct
from tempfile import TemporaryDirectory
from blake3 import blake3

ROOT = Path(__file__).resolve().parent / "corpus" / "combined-checkpoint-v1"
HEADER = b"LATLOG01" + struct.pack("<II", 1, 0)
MAGIC = b"LATCP001"
CHUNK_BYTES = 256 * 1024
GENERATION = "0123456789abcdef0123456789abcdef"
# Public-store test seams: Clock=4242, explicit counter Rng emits owner=1 then r1=2, r2=3.
SECOND_GENERATION = ((4242 << 80) | (3 << 64) | 2).to_bytes(16, "big").hex()
V1 = b"type Ticket {\n  points: int in 0..=5\n}\n"
V2 = b"type Ticket {\n  aaa: bool\n  points: int in 0..=10\n}\n"

def u32(n): return struct.pack("<I", n)
def u64(n): return struct.pack("<Q", n)
def field(b): return u64(len(b)) + b
def hash_bytes(b): return blake3(b).digest()
def crc32c(data):
    crc = 0xffffffff
    for byte in data:
        crc ^= byte
        for _ in range(8): crc = (crc >> 1) ^ (0x82f63b78 if crc & 1 else 0)
    return crc ^ 0xffffffff

def wal_record(lsn, kind, payload):
    covered = u32(len(payload)) + u64(lsn) + bytes([kind]) + payload
    return u32(len(payload)) + u32(crc32c(covered)) + u64(lsn) + bytes([kind]) + payload

def changeset(number, lsn, payload):
    return (b"\1" + number.to_bytes(16, "big") + struct.pack("<QII", 1000 + lsn, 0, 1)
            + bytes(16) + b"\0" + struct.pack("<H", 0) + u32(len(payload)) + payload)

def schema(version, source, table):
    name = b"Ticket"
    return b"\4" + u64(version) + u32(len(source)) + source + u32(1) + u32(table) + u32(len(name)) + name

def catalog_payload(ops, typed=True):
    return b"LCAT" + bytes([2 if typed else 1]) + u32(len(ops)) + b"".join(ops)

def create(table, name): return b"\1" + u32(table) + u32(len(name)) + name
def drop(table): return b"\3" + u32(table)
def typed_value(version, points, aaa=None):
    # Layout names are alphabetical: points in v1; aaa, points in v2.
    cells = bytes([1, 1 if aaa is None else 3])
    if aaa is not None: cells += bytes([aaa])
    cells += struct.pack("<q", points) + u32(0)  # no overflow fields
    return b"LTYP\1" + u64(version) + cells

def row_payload(writes):
    out = b"LROW\1" + u32(len(writes))
    for table, key, value in writes:
        out += u32(table) + u32(len(key)) + key + bytes([value is not None])
        if value is not None: out += u32(len(value)) + value
    return out

def catalog_state(used, versions, lifetimes, schemas):
    out = u64(len(used)) + b"".join(u64(table) for table in used) + u64(len(versions))
    for lsn, tables in versions:
        out += u64(lsn) + u64(len(tables))
        for table, name in tables: out += u64(table) + field(name)
    out += u64(len(lifetimes))
    for table, born, dead in lifetimes: out += u64(table) + u64(born) + u64(dead or 0)
    out += u64(len(schemas))
    for lsn, definition in schemas: out += u64(lsn) + field(definition)
    return out

def row_state(applied, versions):
    out = u64(applied) + u64(len(versions))
    for table, key, lsn, cs, value in versions:
        out += u64(table) + field(key) + u64(lsn) + cs.to_bytes(16, "big") + u64(value is not None)
        if value is not None: out += field(value)
    return out

def kernel_state(through):
    count = through - 1  # LSN1 is a turn fence, all later source records are ChangeSets.
    return (struct.pack("<QII", 1000 + through, 0, 1) + u64(count)
            + b"".join(n.to_bytes(16, "big") for n in range(1, count + 1))
            + u64(1) + u64(9) + u64(2))

def container(tag, through, generation, segments, chunks):
    inventory = bytes(32)
    for first, data in segments:
        part = hash_bytes(f"{first:020}.seg".encode() + u64(len(data)))
        inventory = bytes(a ^ b for a, b in zip(inventory, part))
    manifest = (MAGIC + field(tag) + u64(through) + u64(len(segments))
                + u64(sum(len(data) for _, data in segments)) + inventory
                + u64(sum(map(len, chunks))) + u64(len(chunks))
                + b"".join(u64(len(chunk)) + hash_bytes(chunk) for chunk in chunks))
    name = f"{through+1:020}-{generation}".encode()
    seal = HEADER + wal_record(through+1, 4, MAGIC + field(name) + hash_bytes(manifest))
    return manifest, seal

def generate(target):
    files = {}
    def emit(case, name, data):
        path = Path(case) / (name + ".golden")
        files[path] = data
        files[Path(str(path) + ".toml")] = (f'format = "combined-typed-checkpoint-v1-{name}"\nversion = 1\n'
            f'generator = "formats/generate_combined_checkpoint_golden.py"\ncase = "{case}"\n').encode()
    envelopes = {"envelope-v1-old": typed_value(1, 3), "envelope-v1-same": typed_value(1, 4),
                 "envelope-v2-new": typed_value(2, 8, True), "envelope-v2-same": typed_value(2, 9, False)}
    for case in ["evolved-rows", "invisible-catalog"]:
        if case == "evolved-rows":
            first = catalog_payload([schema(1, V1, 1)])
            second = catalog_payload([schema(2, V2, 1)])
            payloads = [first,
                        row_payload([(1, b"old", envelopes["envelope-v1-old"]), (1, b"same", envelopes["envelope-v1-same"])]),
                        second,
                        row_payload([(1, b"new", envelopes["envelope-v2-new"]), (1, b"same", envelopes["envelope-v2-same"])]),
                        row_payload([(2, b"raw tombstone", None)])]
            catalog = catalog_state([1], [(2, [(1, b"Ticket")]), (4, [(1, b"Ticket")])],
                                    [(1, 2, None)], [(2, first), (4, second)])
            engine = catalog + u64(2) + u64(1) + u64(2) + row_state(6, [
                (1, b"new", 5, 4, envelopes["envelope-v2-new"]),
                (1, b"old", 3, 2, envelopes["envelope-v1-old"]),
                (1, b"same", 3, 2, envelopes["envelope-v1-same"]),
                (1, b"same", 5, 4, envelopes["envelope-v2-same"]), (2, b"raw tombstone", 6, 5, None)])
            tag = b"typed-row-catalog-history-v1"
            emit(case, "schema-v1", first); emit(case, "schema-v2", second)
            for name, data in envelopes.items(): emit(case, name, data)
        else:
            definition = catalog_payload([schema(1, V1, 7)])
            atomic = catalog_payload([schema(1, V1, 7), drop(7)])
            payloads = [atomic, catalog_payload([create(8, b"Other")], typed=False)]
            engine = catalog_state([7, 8], [(2, []), (3, [(8, b"Other")])],
                                   [(7, 2, 2), (8, 3, None)], [(2, definition)])
            tag = b"typed-catalog-history-v1"
            emit(case, "atomic-schema-drop", atomic)
        through = len(payloads) + 1
        prefix = HEADER + wal_record(1, 2, u64(9) + u64(2))
        for number, payload in enumerate(payloads, 1): prefix += wal_record(number+1, 1, changeset(number, number+1, payload))
        state = kernel_state(through) + engine
        chunks = [state[i:i+CHUNK_BYTES] for i in range(0, len(state), CHUNK_BYTES)]
        manifest, seal = container(tag, through, GENERATION, [(1, prefix)], chunks)
        next_manifest, next_seal = container(tag, through+1, SECOND_GENERATION, [(1, prefix), (through+1, seal)], chunks)
        for name, data in [("prefix", prefix), ("engine-state", engine), ("manifest", manifest), ("seal-segment", seal),
                           ("reencoded-manifest", next_manifest), ("reencoded-seal-segment", next_seal)]: emit(case, name, data)
        for i, data in enumerate(chunks): emit(case, f"chunk-{i:04}", data)
    for path, data in files.items():
        dest = target / path; dest.parent.mkdir(parents=True, exist_ok=True); dest.write_bytes(data)
    return files

def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument("--check", action="store_true"); args = parser.parse_args()
    if args.check:
        with TemporaryDirectory() as temp:
            files = generate(Path(temp))
            bad = [str(path) for path, data in files.items() if not (ROOT/path).is_file() or (ROOT/path).read_bytes() != data]
            extra = [str(path.relative_to(ROOT)) for path in ROOT.rglob("*") if path.is_file() and path.relative_to(ROOT) not in files]
            if bad or extra: raise SystemExit(f"combined fixtures differ: {bad}; unexpected: {extra}")
        print(f"combined corpus reproduced exactly ({len(files)//2} binary fixtures)")
    else:
        files = generate(ROOT); print(f"wrote {len(files)//2} independent combined binary fixtures")

if __name__ == "__main__": main()
