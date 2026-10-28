# lattice-testkit

Layer dev. Test support: oracles, generators, invariant checkers, workloads.

Planned modules (Phase 1): `oracle` (in-memory references per engine), `gen` (workload generators and
shrinkers), `invariants` (I1 to I14 checkers), `golden` (golden-file helpers), `fixtures`.
Owner: core owners. Must not: be a normal dependency of any non-dev crate.
