#!/usr/bin/env python3
"""Fresh-process RSS/time/IO evidence; heavyweight dataset runs belong on manual Linux CI.

Construction, recovery and semantic verification execute in different processes. GNU time reports
whole recover-process peak RSS, while the Rust probe reports engine-open elapsed time separately.
The Linux cgroup ceiling includes charged file cache and child processes; it is not renamed RSS.
"""
import argparse
import json
from pathlib import Path
import platform
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
MIB = 1024**2
CASES = [
    ("baseline_minimal", dict(rows=0, versions=0, opaque_records=0, tail_rows=0)),
    ("retained_256", dict(rows=256, versions=4, value_bytes=1024, opaque_records=0)),
    ("retained_1024", dict(rows=1024, versions=4, value_bytes=1024, opaque_records=0)),
    ("retained_4096", dict(rows=4096, versions=4, value_bytes=1024, opaque_records=0)),
    ("payload_4k", dict(opaque_records=64, opaque_bytes=4096)),
    ("payload_256k", dict(opaque_records=64, opaque_bytes=256*1024)),
    ("payload_4m", dict(opaque_records=64, opaque_bytes=4*MIB)),
    ("segments_few", dict(opaque_records=512, opaque_bytes=16*1024, segment_bytes=8*MIB)),
    ("segments_many", dict(opaque_records=512, opaque_bytes=16*1024, segment_bytes=16*1024)),
    ("suffix_zero", dict(opaque_records=64, opaque_bytes=64*1024, tail_rows=0)),
    ("suffix_4096", dict(opaque_records=64, opaque_bytes=64*1024, tail_rows=4096)),
]


def captured(command, stdout_path=None, check=True):
    result = subprocess.run([str(x) for x in command], text=True, capture_output=True)
    if stdout_path:
        stdout_path.write_text(result.stdout)
        stdout_path.with_suffix(stdout_path.suffix + ".stderr").write_text(result.stderr)
    if check and result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {command}\n{result.stderr}\n{result.stdout}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--datasets", required=True, type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--memory-mib", type=int, default=192)
    parser.add_argument("--smoke", action="store_true", help="CI smoke dataset only; still real cgroup/RSS")
    args = parser.parse_args()
    if not 1 <= args.repeats <= 5 or not 64 <= args.memory_mib <= 2048:
        parser.error("repeats 1..5 and memory ceiling 64..2048 MiB required")
    args.probe = args.probe.resolve(); args.output = args.output.resolve(); args.datasets = args.datasets.resolve()
    args.output.mkdir(parents=True, exist_ok=True); args.datasets.mkdir(parents=True, exist_ok=True)
    wrapper = ROOT / "tools/checkpoint_memory_exec.py"
    # A deliberate over-limit child proves the controller actually kills allocations; the measured
    # controller must not silently degrade to a post-hoc RSS assertion or inherited process history.
    calibration = args.output / "ceiling-calibration.json"
    command = ["sudo", "-n", sys.executable, wrapper, "--limit-bytes", 32*MIB, "--output", calibration,
               "--", sys.executable, "-c", "x=bytearray(128*1024*1024); print(len(x))"]
    result = captured(command, args.output / "ceiling-calibration.stdout", check=False)
    evidence = json.loads(calibration.read_text())
    if result.returncode == 0 or evidence["memory_events"].get("oom_kill", 0) < 1:
        raise RuntimeError("memory ceiling calibration did not produce a controller OOM kill")
    report = {"schema_version": 1, "platform": platform.platform(), "python": sys.version,
              "git_head": captured(["git", "rev-parse", "HEAD"]).stdout.strip(),
              "memory_ceiling_mib": args.memory_mib, "repeats": args.repeats,
              "cache_conditions": "fresh processes; shared warm/uncontrolled OS page cache; no cache drop; alternating mode order",
              "measurement_scope": "GNU time whole recover-process peak RSS KiB; engine open_ns excludes sample checks; setup/verify separate",
              "ceiling_scope": "private Linux cgroup v2 process tree plus charged file cache; swap disabled; actual OOM calibration",
              "ceiling_calibration": evidence, "cases": []}
    cases = [("smoke", dict(rows=8, versions=2, opaque_records=4, opaque_bytes=4096, tail_rows=2))] if args.smoke else CASES
    for name, parameters in cases:
        dataset = args.datasets / name
        prepare = [args.probe, "prepare", "--dir", dataset]
        for key, value in parameters.items(): prepare.extend(["--" + key.replace("_", "-"), value])
        meta = json.loads(captured(prepare, args.output / f"{name}-prepare.json").stdout)
        case = {"name": name, "dataset": meta, "runs": []}
        for repeat in range(args.repeats):
            modes = ["full", "checkpoint"] if repeat % 2 == 0 else ["checkpoint", "full"]
            for mode in modes:
                stem = f"{name}-{mode}-{repeat}"; limit = args.output / f"{stem}-cgroup.json"; timing = args.output / f"{stem}-time.txt"
                command = ["sudo", "-n", sys.executable, wrapper, "--limit-bytes", args.memory_mib*MIB, "--output", limit,
                           "--", "/usr/bin/time", "-f", "%M\n%e", "-o", timing,
                           args.probe, "recover", "--dir", dataset, "--mode", mode]
                result = captured(command, args.output / f"{stem}.json")
                metrics = json.loads(result.stdout); cgroup = json.loads(limit.read_text())
                rss, elapsed = timing.read_text().strip().splitlines()
                metrics.update({"repeat": repeat, "peak_rss_kib": int(rss), "process_wall_seconds": float(elapsed), "cgroup": cgroup})
                if not cgroup["limit_enforced"] or cgroup["memory_events"].get("oom_kill", 0) or cgroup["child_exit_code"]:
                    raise RuntimeError(f"memory ceiling failure: {stem}: {cgroup}")
                case["runs"].append(metrics)
        # Full historical index checks and full WAL scrub deliberately run outside timed/RSS children.
        case["verification"] = json.loads(captured([args.probe, "verify", "--dir", dataset], args.output / f"{name}-verify.json").stdout)
        case["summary"] = {mode: {"median_open_ms": statistics.median(r["open_ns"] for r in case["runs"] if r["mode"] == mode)/1e6,
                                  "median_peak_rss_kib": statistics.median(r["peak_rss_kib"] for r in case["runs"] if r["mode"] == mode),
                                  "max_peak_rss_kib": max(r["peak_rss_kib"] for r in case["runs"] if r["mode"] == mode)} for mode in ["full", "checkpoint"]}
        report["cases"].append(case)
        (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
        print(name, json.dumps(case["summary"]), flush=True)
    lines = ["# Checkpoint recovery measurement", "", f"Commit: `{report['git_head']}`", "", report["cache_conditions"], "",
             f"Enforced cgroup ceiling: {args.memory_mib} MiB, swap disabled. Over-limit calibration produced an OOM kill.", "",
             "RSS is whole fresh recover-process peak; cgroup peak also counts charged cache. Retained histories/indexes/dedupe grow.", "",
             "| Dataset | Mode | Prefix bytes / files | Suffix bytes | State bytes | Median open ms | Peak RSS KiB (max) |", "|---|---|---|---|---|---|---|"]
    for case in report["cases"]:
        meta = case["dataset"]
        for mode in ["full", "checkpoint"]:
            runs = [r for r in case["runs"] if r["mode"] == mode]; summary = case["summary"][mode]
            lines.append(f"| {case['name']} | {mode} | {meta['prefix_bytes']} / {meta['prefix_files']} | {meta['suffix_bytes']} | {meta['checkpoint_state_bytes']} | {summary['median_open_ms']:.3f} | {summary['max_peak_rss_kib']} |")
    lines.extend(["", "## Checkpoint I/O and retained state", "",
                  "| Dataset | Row / index versions | Committed IDs | Prefix reads | len calls | WAL entries visited | Largest read | Max cgroup peak bytes |",
                  "|---|---|---|---|---|---|---|---|"])
    for case in report["cases"]:
        meta = case["dataset"]
        runs = [r for r in case["runs"] if r["mode"] == "checkpoint"]
        first = runs[0]
        lines.append(f"| {case['name']} | {meta['retained_row_versions']} / {meta['retained_index_versions']} | {meta['kernel_committed_ids']} | {first['prefix_read_bytes']} | {first['len_calls']} | {first['wal_directory_entries']} | {first['largest_read']} | {max(r['cgroup']['memory_peak_bytes'] for r in runs)} |")
    (args.output / "summary.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__": main()
