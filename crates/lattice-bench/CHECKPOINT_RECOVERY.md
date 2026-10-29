# Draft checkpoint recovery measurements

This is the measurement protocol for draft RFC0002/PR156, not format acceptance or a claim that
P2-08/#118/#108 are complete. The review hold remains 2026-11-05 00:44:13 UTC plus explicit acceptance.

## Reproduce on manual Linux CI

The existing registered `io-probe.yml` is the dispatch carrier. Select the PR branch and set
`checkpoint_recovery=true`; the ordinary commit/runtime measurements are skipped only for that
explicit selection. The reusable `checkpoint-recovery.yml` job builds the probe in release mode,
checks independent fixtures and engine correctness, then runs the measurement script. Default
scheduled/ordinary manual runtime behavior is unchanged.

```
gh workflow run io-probe.yml --ref feat/engine-checkpoint-recovery \
  -f checkpoint_recovery=true -f checkpoint_repeats=3 -f checkpoint_memory_mib=192
```

Review the scripts/workflow before dispatch. Heavy measurements belong on the ephemeral CI runner,
not the development laptop. A local tiny functional smoke is not performance evidence.

Artifacts include `results.json`, every process's raw stdout/stderr, GNU-time output, cgroup memory
metrics, setup metadata, separate semantic verification output, runner CPU/kernel/filesystem context,
and `summary.md`. Preserve the workflow run URL, exact commit SHA and artifact together when publishing
results. The first measured run and its audited results are recorded below.

## Processes and what the numbers mean

`checkpoint_probe` is a host-only example in `lattice-bench` (engines remain a dev-dependency).
`prepare` constructs a new dataset, checkpoints, adds the suffix, and exits. Each `recover` invocation
is a fresh process. GNU `/usr/bin/time` reports peak RSS in KiB and whole-process elapsed seconds;
the Rust probe separately reports engine-open nanoseconds, excluding the subsequent bounded sample
checks. RSS still includes process startup, JSON configuration and those small sample allocations.
There is no inherited dataset-construction peak. `verify` runs in another process and checks every
configured historical index snapshot, every source key/value byte and tombstone at every snapshot,
every suffix row and exact live row counts, kernel dedupe/fences and a full
WAL scrub. Its allocations/IO are excluded from recovery measurements.

The full-replay baseline uses the same RowStore/kernel/index reconstruction but deliberately makes
only the optional checkpoint directory invisible through an Io adapter. It does not remove or hide
WAL segments, replace IDs, skip the kernel decoder, or keep only latest rows. The snapshot path
asserts the expected source LSN, zero prefix-segment reads and zero collecting directory calls.
Both modes must agree with historical samples and the separate full-history/index/scrub verification.

The external controller puts the measured process tree (GNU time plus Rust child) in a private Linux
cgroup v2 with `memory.max` set to the selected byte ceiling and `memory.swap.max=0`. The Python
controller stays outside that group, removes it afterward, and never changes global memory/security
settings. The job first launches a deliberately over-limit allocation in a separate 32 MiB group
and requires an actual `oom_kill` event. Every measured run must exit successfully with no OOM kill;
missing/unusable controller files fail the experiment rather than degrading to a post-hoc claim.
`memory.peak` includes charged page cache and the entire child tree; it is not RSS. GNU-time RSS is
reported separately. Passing is finite evidence for these exact datasets/configurations/ceiling,
not a proof of an asymptotic resident-memory bound for arbitrary databases.

The cache condition is **fresh processes with shared warm/uncontrolled filesystem cache**. No global
cache drop is performed. Modes alternate order across repetitions to reduce order bias. Do not label
these cold-cache timings or compare absolute values across unrelated runner hardware as regressions.
The selected ceiling and all configuration values appear in the report; defaults are three repeats
per mode and 192 MiB per process tree, with swap disabled.

## Dataset axes

| Series | Variation | Held constant |
| --- | --- | --- |
| baseline | No row versions, opaque IDs or suffix; one real turn fence | Minimal executable/kernel/engine setup |
| retained state | 256 / 1024 / 4096 keys, four versions, 1024-byte row values | No opaque prefix; same 32-row suffix and segment target |
| opaque prefix bytes | 64 records with 4 KiB / 256 KiB / 4 MiB opaque payloads | **64 distinct acknowledged IDs**, 128 keys/two versions, same suffix/segment target |
| prefix file count | Same 512 records ×16 KiB payload; 8 MiB vs16 KiB segment target | Same distinct-ID count, row history and payload bytes |
| suffix growth | Zero vs4096 suffix rows, 1024-byte values | Same 64×64 KiB opaque prefix and source row history |

The opaque-prefix series has a constant **count** of unique acknowledged ChangeSet IDs; it does not
resubmit duplicate IDs or bypass deduplication. Payloads are ignored by the row codec but are real
checksummed WAL changesets decoded by the kernel and retained in its seen set. The fixture/dataset
encoder never substitutes a latest-only snapshot. Final updates delete every seventh source key
when multiple versions exist, preserving historical values and tombstones. The deterministic index
extracts the version field, so every update changes index history too.

Setup metadata records actual retained row/index version counts, tombstone counts, kernel committed
ID cardinality, source and suffix LSNs, actual prefix bytes/file count, actual suffix bytes/files,
checkpoint bytes/chunk count and largest physical WAL segment. The segment rotation target is soft:
a large record can make the actual segment larger than the target. Interpret read-buffer limits
against the reported actual largest segment, not merely the requested target.

## Interpretation and remaining scope

Retained rows, historical index entries, catalog versions and dedupe IDs necessarily grow. The
retained-state series is expected to show that growth; do not call total memory constant. The
opaque-prefix series isolates extra replay bytes without increasing ID cardinality. Prefix metadata
validation still visits segment names and reads lengths in O(prefix file count). Raw results include
`len_calls` and visited directory-entry counts so this work is visible; zero prefix-byte reads does
not mean zero prefix metadata work. Tail segment names and reconstructed suffix state also grow.

Snapshot reads are individually bounded at 256 KiB, but row-value allocations, manifest metadata,
engine state, index rebuilding and a whole actual WAL segment are separate costs. Maximum read
request size alone is not allocator-peak/RSS evidence. The measured fresh-process RSS and enforced
cgroup result supply that experimental evidence within the declared configuration.

The sealed checkpoint is authoritative for unread prefix state. Same-length prefix bit rot is
outside fast-open content verification; the separate full scrub audits retained WAL contents.
No WAL is pruned. Typed schema/layout recovery is a separate combined-tag integration draft, not
silently encoded in the raw/legacy tags tested here.

The full-replay comparison here uses the streaming RowStore recovery path. The separate legacy
`Kernel::open` API intentionally returns and therefore collects all recovered records; this probe
does not establish a bound for that API or close #108 on its behalf. The separate verification
process may use that collecting API, and its peak is explicitly excluded from reported RSS.

## Measured run: 2026-10-29

[Workflow run 40797913195](https://github.com/moncifem/latticedb/actions/runs/40797913195) succeeded on
commit `1d7035092b192a6f9104c7aa1930a65d7390a5e0`. The existing carrier skipped unrelated commit/runtime
measurements as intended. Runner: Ubuntu 24.04, Linux 7.0.0-1017-azure x86_64, four AMD EPYC 7763
vCPUs, ext4, Python 3.12.3. Preserve the raw [results](evidence/checkpoint-recovery/2026-10-29/results.json),
[complete generated tables](evidence/checkpoint-recovery/2026-10-29/summary.md),
[calibration](evidence/checkpoint-recovery/2026-10-29/ceiling-calibration.json) and
[run/artifact provenance](evidence/checkpoint-recovery/2026-10-29/provenance.json); runner context is
stored beside them. The uploaded artifact retains each individual raw stdout, time and cgroup file.

The audit checked all **11 datasets, 66 fresh recovery processes (three repeats per mode), and eleven
complete semantic verifications**. Aggregate JSON agrees with every raw trial stdout, GNU-time file
and cgroup file. All measured processes exited successfully with the enforced **192 MiB** cgroup
limit, zero swap and no OOM kill. The separate 32 MiB calibration group killed the deliberate
128 MiB allocation with signal 9 and exactly one `oom_kill` event. Maximum measured RSS across all
cases was **27,632 KiB full replay / 26,844 KiB checkpoint**. Maximum cgroup peaks were 26,411,008 /
25,882,624 bytes respectively. Cgroup accounting is not a sum of RSS: shared pages can remain charged
to other groups, so its peak can be below a child's RSS. Both raw quantities remain distinct.

Selected results use median engine-open time and **maximum**, not median, RSS over three trials:

| Dataset | Full open ms | Checkpoint open ms | Full peak RSS KiB | Checkpoint peak RSS KiB |
| --- | --- | --- | --- | --- |
| Minimal baseline | 0.461 | 0.602 | 2,612 | 2,684 |
| 256 keys ×4 retained versions | 6.755 | 3.514 | 5,068 | 4,196 |
| 1024 keys ×4 retained versions | 24.074 | 10.900 | 10,336 | 8,820 |
| 4096 keys ×4 retained versions | 88.114 | 43.388 | 27,632 | 26,844 |
| 268,528,165-byte opaque prefix | 854.850 | 1.163 | 18,816 | 2,860 |
| 518 prefix files | 32.186 | 2.824 | 2,928 | 2,876 |
| 4096 suffix rows /4,616,358 suffix bytes | 33.582 | 21.879 | 9,992 | 9,928 |

The retained-state series increases encoded source state from 1,092,376 to 17,483,248 bytes, with
final retained row/index version counts growing from 1056/1755 to 16416/28086. Checkpoint RSS grows
from 4,196 to 26,844 KiB accordingly: **total resident state is not constant**.

The three payload-size datasets hold the same source row history and count of distinct acknowledged
IDs (64 opaque IDs, 256 source-row IDs, plus 32 suffix-row IDs =352 final IDs), and the same 82,088-byte
encoded source snapshot. Prefix bytes grow from 353,829 to 268,528,165; checkpoint trials read zero
prefix bytes and remain near one millisecond on this warm-cache runner. That is evidence of skipped
replay work under this exact workload, not a cold-cache or production latency promise.

Prefix metadata work remains visible. With constant opaque payload bytes/ID count and different
segment targets, prefix file count grows **2→518**, length calls **5→521**, and visited WAL directory
entries **10→1046**. Median checkpoint open time grows **1.020→2.824 ms**. The small physical byte
increase is the additional 16-byte segment headers. All fast trials obey the measured length-call
formula `prefix_files + 2 × chunk_count + 1`; this remains O(prefix file count), not constant work.

The suffix series grows checkpoint maximum RSS **2,856→9,928 KiB** and open time **0.824→21.879 ms**
as actual suffix bytes grow from 134 to 4,616,358. Its largest fast-path read is **2,097,481 bytes**,
coming from the ordinary WAL suffix segment. Snapshot chunk reads remain bounded at 262,144 bytes;
that bound does not apply to whole WAL segments. The largest actual segment in the complete dataset
matrix is 8,393,119 bytes, slightly above the 8 MiB soft rotation target.

These measurements establish finite configured-workload evidence for the streaming RowStore path.
They do not bound the separate collecting `Kernel::open` API, arbitrary future retained state, typed
combined-tag recovery, or cold-cache latency. Full prefix content integrity still requires scrub.
RFC acceptance, the seven-day hold, #118 and #108 remain open.
