# Phase 10: cluster and hardening

## Goal

Make Lattice survive failures and scale beyond one machine, and prove it with chaos testing, external security review and disaster recovery drills.

## Scope

**In:** Raft-replicated shards, replicated and erasure-coded blob segments, federation with signed deltas, backup and restore, point-in-time recovery, online upgrade, multi-tenant isolation and quotas, performance tuning, external security review and penetration test, chaos and soak, capacity model.

**Out:** new product features.

## Requirements

| ID | Requirement | Verified |
|---|---|---|
| P10-01 | Shard map and Raft replication for the log and row engines; membership changes; leader election; the commit protocol TLA+ model extended to replication | S, I |
| P10-02 | **Linearizable writes per shard** and snapshot-consistent reads; cross-shard ChangeSets either forbidden by design or specified with a protocol and model | S, I |
| P10-03 | Blob segments replicated or erasure coded (Reed-Solomon) with repair; any allowed number of lost shards is tolerated in simulation | S, P |
| P10-04 | Federation: signed deltas exchanged between instances with a defined trust model; replay and tamper tests | S, T |
| P10-05 | Backup and restore with verification; point-in-time recovery to a ChangeSet or timestamp; restore drills automated | T |
| P10-06 | Online rolling upgrade with mixed-version nodes within the compatibility policy; downgrade within one minor version | S, T |
| P10-07 | Tenant isolation: separate keys, quotas, noisy-neighbour limits, per-tenant data residency tags | T, S |
| P10-08 | Simulation extended with node crash, restart, partition, clock skew and slow disk across a multi-node cluster; I1 to I14 checked | S |
| P10-09 | Jepsen-style chaos on real hardware: partitions, process kills, disk faults; Elle-checked histories | T |
| P10-10 | Soak: 30 days with a synthetic mixed workload; scrubbers clean; no memory growth beyond budget | T |
| P10-11 | External security review and penetration test of IdP, policy, API, S3 and plugin host; all high findings fixed before GA | I |
| P10-12 | Capacity model and published sizing guide derived from benchmarks | B, I |
| P10-13 | Operational runbooks and `lattice doctor` cluster checks; SLOs and alerts defined | I, D |

## Tests, evals and benchmarks added

- Multi-node simulation profiles added to the nightly; planted bugs extended with replication and repair bugs.
- Chaos weekly on real hardware; soak monthly.
- Disaster recovery drill (node loss, region loss, corrupted disk, operator error) with timings recorded.
- Benchmarks: scale-out throughput, failover time, repair time, upgrade impact.

## Exit criteria

| # | Criterion |
|---|---|
| E10-1 | I1 to I14 hold across 1,000,000 multi-node seeds per night for seven nights |
| E10-2 | Chaos runs for four consecutive weeks with no invariant violation |
| E10-3 | 30-day soak clean |
| E10-4 | Restore and PITR drills succeed within the recovery targets set at the gate |
| E10-5 | External review complete, no unresolved high findings |
| E10-6 | Rolling upgrade tested across the supported version window |
| E10-7 | Decision recorded on whether cluster mode ships in 1.0 or in a 1.x release, based on the evidence |

## Depends on, risks

Depends on Phases 8 and 9. Risks: distributed correctness is the hardest work in the project (models first, simulation always, scope limited to per-shard linearizability); cross-shard transactions (avoid by design if possible); cost of real-hardware chaos (scheduled weekly, not per PR).
