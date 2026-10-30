# Verification

Lattice tries to show that each layer holds before the next one is built on it. The evidence below is listed
with its status. The phase gate pages are the source of truth, and this page only summarises them. Phase 1's
gate is open, so none of these numbers is a sign-off.

## The evidence

| Layer | What runs | Where it stands |
|---|---|---|
| Unit and property tests | Every pull request runs the property tests at 10,000 cases. Nightly runs 1,000,000 cases for the core, ABI and schema crates | Met for the cases above ([Phase 1 gate, E1-5](../gates/phase-1.md)) |
| Deterministic simulator | Seeded runs of crashes, torn writes, disk errors and bit rot across fault profiles, including a `non-prefix` profile for out-of-order power loss | Identical traces across repeated runs are met ([E1-1](../gates/phase-1.md)). The nightly volume is still accumulating clean nights ([E1-2](../gates/phase-1.md)) |
| Power-cut sweep | Power is cut at every storage operation of a fixed set of workloads. Every acknowledged change set must be recovered | Met ([E1-4](../gates/phase-1.md)) |
| Planted bugs | Switchable defects, enabled with the `planted-bugs` feature of `lattice-core`. Each one must make the checks fail | Every kernel planted bug is detected ([E1-3](../gates/phase-1.md)). The other switchable defects are checked by focused tests |
| Fuzzing | Targets under [`fuzz/`](../../fuzz). Scheduled runs are short nightly, longer weekly, and a manual campaign can run for 24 hours | A manual campaign ran the core targets with no crash ([E1-6](../gates/phase-1.md)). Scheduled runs keep extending the corpus |
| Mutation testing | Mutants are generated and the tests must kill them | No verified complete score yet. The campaigns so far lost shard outcomes, so no number is reported ([E1-7](../gates/phase-1.md)) |
| Formal models | TLA+ models in [`tla/`](../../tla) for commit, fencing, hybrid logical clocks, set convergence and tree moves. Negative configurations must fail when a protection is removed | Run by [`tla/check-all.sh`](../../tla/check-all.sh) |
| Bounded proofs | Kani harnesses for the change-feed cursor and event decoders, the sync frame decoder, the SDK framer and the tree cycle rule | Bounded, not full proofs. Some harnesses are not proven yet. See the [Kani page](../verification.md) |
| Memory and undefined behaviour | Miri, AddressSanitizer and Loom jobs in CI | Run in CI |
| SQL reference check | sqllogictest scripts in `crates/lattice-testkit/tests/slt`, compared with PostgreSQL through a reference script | Runs nightly |

## What the evidence does not establish

- Real power loss. Durability is tested with the simulator and with process kills. Linux real-disk runtime
  numbers are still pending ([ADR 0011](../adr/0011-engine-runtime.md)).
- Anything the simulator does not model. Its checks cover the invariants that the Phase 1 gate names, and no more.
- Larger inputs than a bounded proof covers. The Kani proofs are for small inputs, and the property tests and
  fuzz targets cover the rest.
- A complete mutation score. Until one is verified, do not quote a number.
- The phase exit criteria. The owner signs each gate. No page claims a gate passed before that.

## Reproduce the basics locally

These are the commands the README lists. The full pull-request gate is heavy, so it runs in CI:

```bash
cargo xtask sim --seeds 1000    # replay crashes across the fault profiles
cargo xtask ci                  # the full pull request gate
```

The [gate pages](../gates) record every measured number with the commit it came from. The
[roadmap](../roadmap.md) says what each phase still needs.
