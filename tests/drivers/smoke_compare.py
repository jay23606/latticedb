#!/usr/bin/env python3
"""Compares one driver smoke run with its expected output (issue 576).

    smoke_compare.py <expected.jsonl> <known.jsonl> <actual.jsonl>

expected.jsonl holds what PostgreSQL does for every step, written by hand. known.jsonl pins, for the
steps that latticed does not yet pass, the output latticed gives today, with the issue that tracks
it. A pinned step passes only while latticed still gives exactly the pinned output; if it matches
PostgreSQL, or changes to anything else, the run fails. Every other step must equal its expected
record. The step sequence must match, and the expected output must cover the required tags, so a
script that skips a category cannot pass.
"""

import json
import sys

REQUIRED_TAGS = ["parameter", "null", "bool", "int", "text", "error"]


def fail_usage():
    print("usage: smoke_compare.py <expected.jsonl> <known.jsonl> <actual.jsonl>", file=sys.stderr)
    return 2


def load(path):
    records = []
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, 1):
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise SystemExit(f"{path}:{number}: not JSON: {exc}")
    return records


def check(expected, known_entries, actual):
    failures = []
    passed = []
    xfailed = []

    tags = set()
    for record in expected:
        tags.update(record.get("tags", []))
        if "error" in record.get("tags", []) and "sqlstate" not in record:
            failures.append(f"error step without a sqlstate in expected output: {record.get('step')}")
    for tag in REQUIRED_TAGS:
        if tag not in tags:
            failures.append(f"expected output does not cover the tag '{tag}'")

    names = [record["step"] for record in expected]
    known = {}
    for entry in known_entries:
        name = entry.get("step")
        if name in known:
            failures.append(f"known.jsonl lists {name} twice")
        if "issue" not in entry or "latticed" not in entry:
            failures.append(f"known.jsonl entry {name} needs an issue and the pinned latticed output")
        if name not in names:
            failures.append(f"known.jsonl entry {name} is not a step of expected.jsonl")
        known[name] = entry

    got = [record.get("step") for record in actual]
    if got != names:
        failures.append(f"step sequence differs: expected {names}, got {got}")
        return failures, passed, xfailed

    for want, have in zip(expected, actual):
        name = want["step"]
        if name in known:
            entry = known[name]
            if have == want:
                failures.append(
                    f"{name} now matches PostgreSQL: move it out of known.jsonl and drop its issue note ({entry['issue']})"
                )
            elif have == entry["latticed"]:
                xfailed.append(f"{name} ({entry['issue']})")
            else:
                failures.append(
                    f"{name} differs from its pinned latticed output: expected {json.dumps(want, sort_keys=True)}, "
                    f"pinned {json.dumps(entry['latticed'], sort_keys=True)}, got {json.dumps(have, sort_keys=True)}"
                )
        elif have == want:
            passed.append(name)
        else:
            failures.append(
                f"{name}: expected {json.dumps(want, sort_keys=True)}, got {json.dumps(have, sort_keys=True)}"
            )
    return failures, passed, xfailed


def main(argv):
    if len(argv) != 4:
        return fail_usage()
    expected = load(argv[1])
    known = load(argv[2])
    actual = load(argv[3])
    if not expected:
        print("expected output is empty", file=sys.stderr)
        return 1
    failures, passed, xfailed = check(expected, known, actual)
    for name in passed:
        print(f"ok      {name}")
    for name in xfailed:
        print(f"xfail   {name}")
    if failures:
        for message in failures:
            print(f"FAILED  {message}", file=sys.stderr)
        return 1
    print(f"{len(passed)} passed, {len(xfailed)} known failing, {len(expected)} steps")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
