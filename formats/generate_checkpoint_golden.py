#!/usr/bin/env python3
"""Independent RFC0002 encoder; never invokes Rust/checkpoint production code.

Requires Python >=3.11 and the external standard BLAKE3 hash library:
  python3 -m pip install --only-binary=:all: blake3==1.0.11
  python3 formats/generate_checkpoint_golden.py [--check]
Only hashing is delegated; WAL CRC/framing, kernel/engine state, manifest and seal are encoded here.
"""
import argparse
import struct
from pathlib import Path
from tempfile import TemporaryDirectory
from blake3 import blake3

ROOT = Path(__file__).resolve().parent / "corpus" / "checkpoint-v1"
GENERATION = "0123456789abcdef0123456789abcdef"
MAGIC = b"LATCP001"
HEADER = b"LATLOG01" + struct.pack("<II", 1, 0)
CHUNK = 256 * 1024

def u64(n): return struct.pack("<Q", n)
def field(b): return u64(len(b)) + b
def digest(b): return blake3(b).digest()
def crc32c(b):
    crc = 0xffffffff
    for byte in b:
        crc ^= byte
        for _ in range(8): crc = (crc >> 1) ^ (0x82f63b78 if crc & 1 else 0)
    return crc ^ 0xffffffff

def record(lsn, kind, payload):
    length = struct.pack("<I", len(payload))
    covered = length + u64(lsn) + bytes([kind]) + payload
    return length + struct.pack("<I", crc32c(covered)) + u64(lsn) + bytes([kind]) + payload

def changeset(number, lsn, payload):
    return (b"\1" + number.to_bytes(16, "big") + struct.pack("<QII", 1000 + lsn, 0, 1)
            + bytes(16) + b"\0" + struct.pack("<HI", 0, len(payload)) + payload)

def rowops(writes):
    out = b"LROW\1" + struct.pack("<I", len(writes))
    for key, value in writes:
        out += struct.pack("<II", 7, len(key)) + key + bytes([value is not None])
        if value is not None: out += struct.pack("<I", len(value)) + value
    return out

def catops(ops):
    out = b"LCAT\1" + struct.pack("<I", len(ops))
    for kind, table, name in ops:
        out += struct.pack("<BI", kind, table)
        if kind != 3: out += struct.pack("<I", len(name)) + name
    return out

def row_state():
    versions = [(b"", 3, 2, b"zero"), (b"k", 2, 1, b"one"),
                (b"k", 3, 2, b"two"), (b"k", 4, 3, None)]
    out = u64(4) + u64(len(versions))
    for key, lsn, cs, value in versions:
        out += u64(7) + field(key) + u64(lsn) + cs.to_bytes(16, "big") + u64(value is not None)
        if value is not None: out += field(value)
    return out

def cat_state():
    out = u64(2) + u64(7) + u64(8) + u64(3)
    for lsn, table, name in [(2, 7, b"before"), (3, 7, b"after"), (4, 8, b"next")]:
        out += u64(lsn) + u64(1) + u64(table) + field(name)
    return out

def generate(target):
    files = {}
    def emit(case, name, data):
        path = Path(case) / (name + ".golden")
        files[path] = data
        files[Path(str(path) + ".toml")] = (f'format = "engine-checkpoint-v1-{name}"\nversion = 1\n'
             f'generator = "formats/generate_checkpoint_golden.py"\ncase = "{case}"\n').encode()
    for case in ["empty", "row", "catalog", "split"]:
        through = 0 if case in ["empty", "split"] else 4
        if case == "row":
            payloads = [rowops([(b"k", b"one")]), rowops([(b"k", b"two"), (b"", b"zero")]), rowops([(b"k", None)])]
            tag, engine = b"raw-row-history-v1", row_state()
        elif case == "catalog":
            payloads = [catops([(1, 7, b"before")]), catops([(2, 7, b"after")]), catops([(3, 7, b""), (1, 8, b"next")])]
            tag, engine = b"legacy-catalog-history-v1", cat_state()
        elif case == "empty":
            payloads, tag, engine = [], b"raw-row-history-v1", u64(0) + u64(0)
        else:
            payloads, tag, engine = [], b"test-split-v1", bytes(CHUNK + 31)
        prefix = HEADER + record(1, 2, u64(9) + u64(2)) if through else b""
        for number, payload in enumerate(payloads, 1): prefix += record(number + 1, 1, changeset(number, number + 1, payload))
        kernel = (struct.pack("<QII", 1004, 0, 1) + u64(3)
                  + b"".join(n.to_bytes(16, "big") for n in [1, 2, 3]) + u64(1) + u64(9) + u64(2)) if through else bytes(16) + u64(0) + u64(0)
        state = kernel + engine
        chunks = [state[i:i+CHUNK] for i in range(0, len(state), CHUNK)]
        prefix_hash = digest(b"00000000000000000001.seg" + u64(len(prefix))) if through else bytes(32)
        manifest = (MAGIC + field(tag) + u64(through) + u64(bool(through)) + u64(len(prefix)) + prefix_hash
                    + u64(len(state)) + u64(len(chunks))
                    + b"".join(u64(len(chunk)) + digest(chunk) for chunk in chunks))
        basename = f"{through+1:020}-{GENERATION}".encode()
        seal = HEADER + record(through+1, 4, MAGIC + field(basename) + digest(manifest))
        emit(case, "prefix", prefix); emit(case, "engine-state", engine); emit(case, "manifest", manifest); emit(case, "seal-segment", seal)
        for i, chunk in enumerate(chunks): emit(case, f"chunk-{i:04}", chunk)
    for path, data in files.items():
        out = target / path; out.parent.mkdir(parents=True, exist_ok=True); out.write_bytes(data)
    return files

def main():
    parser = argparse.ArgumentParser(description=__doc__); parser.add_argument("--check", action="store_true"); args = parser.parse_args()
    if args.check:
        with TemporaryDirectory() as d:
            files = generate(Path(d))
            differences = [str(p) for p, b in files.items() if not (ROOT / p).is_file() or (ROOT / p).read_bytes() != b]
            unexpected = [str(p.relative_to(ROOT)) for p in ROOT.rglob("*") if p.is_file() and p.relative_to(ROOT) not in files]
            if differences or unexpected: raise SystemExit(f"corpus differs: {differences}; unexpected: {unexpected}")
        print(f"checkpoint corpus reproduced exactly ({len(files)//2} binary fixtures)")
    else:
        files = generate(ROOT); print(f"wrote {len(files)//2} independent binary fixtures")

if __name__ == "__main__": main()
