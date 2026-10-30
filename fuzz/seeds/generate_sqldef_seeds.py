#!/usr/bin/env python3
"""Seeds for the sqldef_decode fuzz target, framed by the independent golden generator (no Rust codec).

Each seed is one selector byte read by fuzz_targets/sqldef_decode.rs (even: definition body, odd:
definition block) followed by the payload. --check verifies tracked bytes without writing them.
"""
import argparse
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "golden", ROOT.parent.parent / "formats" / "generate_sql_definition_golden.py"
)
GOLDEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GOLDEN)

BODY_REJECTS = [
    "version",
    "zero-columns",
    "uppercase-name",
    "default-numeric-trailing-zero",
    "pk-nullable",
    "sixteen-uniques",
    "trailing-byte",
    "default-length-past-end",
    "column-count-past-end",
]
BLOCK_REJECTS = ["empty", "not-ascending", "duplicate-id", "table-id-over-limit", "body-over-64kib", "count-past-end"]


def seeds():
    out = {}
    for name, data in GOLDEN.ok_bodies().items():
        out[f"sqldef_decode/body-{name}"] = b"\x00" + data
    rejects = GOLDEN.reject_bodies()
    for name in BODY_REJECTS:
        out[f"sqldef_decode/body-reject-{name}"] = b"\x00" + rejects[name]
    for name, data in GOLDEN.ok_blocks().items():
        out[f"sqldef_decode/block-{name}"] = b"\x01" + data
    rejects = GOLDEN.reject_blocks()
    for name in BLOCK_REJECTS:
        out[f"sqldef_decode/block-reject-{name}"] = b"\x01" + rejects[name]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = seeds()
    for name, value in expected.items():
        path = ROOT / name
        if args.check:
            if not path.is_file() or path.read_bytes() != value:
                raise SystemExit(f"seed differs: {name}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value)
    print(f"{len(expected)} independent sqldef seeds " + ("verified" if args.check else "written"))


if __name__ == "__main__":
    main()
