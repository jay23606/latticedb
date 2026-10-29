#!/usr/bin/env python3
"""Run one child in a private Linux cgroup v2 with an enforced memory/swap limit.

Invoked through passwordless sudo only on an explicitly dispatched ephemeral CI runner. The
controller process stays outside the group; only the measured process and its children are charged.
Failure to create/enforce/read the controller is fatal, never downgraded to a measurement claim.
"""
import argparse
import json
import os
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--limit-bytes", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or args.limit_bytes < 16 * 1024**2:
        parser.error("command and at least 16 MiB limit required")
    if sys.platform != "linux" or os.geteuid() != 0:
        parser.error("Linux cgroup v2 root controller required (CI only)")
    root = Path("/sys/fs/cgroup")
    if "memory" not in (root / "cgroup.controllers").read_text().split():
        raise RuntimeError("cgroup v2 memory controller unavailable")
    group = root / f"lattice-checkpoint-{os.getpid()}"
    group.mkdir()
    try:
        (group / "memory.max").write_text(str(args.limit_bytes))
        (group / "memory.swap.max").write_text("0")
        actual_limit = (group / "memory.max").read_text().strip()
        actual_swap = (group / "memory.swap.max").read_text().strip()
        if actual_limit != str(args.limit_bytes) or actual_swap != "0":
            raise RuntimeError("memory controller did not retain requested limits")
        pid = os.fork()
        if pid == 0:
            try:
                (group / "cgroup.procs").write_text(str(os.getpid()))
                os.execvp(command[0], command)
            except BaseException as error:
                print(f"cgroup child failed: {error}", file=sys.stderr, flush=True)
                os._exit(125)
        _, status = os.waitpid(pid, 0)
        code = os.waitstatus_to_exitcode(status)
        events = {key: int(value) for key, value in
                  (line.split() for line in (group / "memory.events").read_text().splitlines())}
        result = {"memory_max_bytes": int(actual_limit), "memory_swap_max_bytes": int(actual_swap),
                  "memory_peak_bytes": int((group / "memory.peak").read_text()),
                  "memory_events": events, "child_exit_code": code,
                  "controller": "linux-cgroup-v2", "limit_enforced": True}
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        return code if code >= 0 else 128 - code
    finally:
        group.rmdir()


if __name__ == "__main__":
    raise SystemExit(main())
