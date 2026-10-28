# Conventions

Authoritative source for naming, errors, versioning and review rules. Spec: `docs/spec/00-repo-and-conventions.md`.

## Naming

| Thing | Rule |
|---|---|
| Crates | `lattice-<name>`, binary `latticed` |
| Types | `UpperCamelCase`; ids are `Id<T>`; errors are `<Crate>Error` |
| Modules, functions | `snake_case` |
| Features | `kebab-case`, additive only |
| Metrics | `lattice_<layer>_<thing>_<unit>` |
| Tracing targets | the crate name; write-path spans carry `txn_id` |
| Config keys | `snake_case`, documented, with defaults |
| On-disk and wire enums | explicit discriminants, `#[non_exhaustive]`, never reordered |
| Invariant tests | `inv_i<N>_<what>` |

## Errors, logging, secrets

Per-crate `thiserror` enums, no `anyhow` in core, engines or service. Errors map onto the taxonomy
`Retryable`, `Rejected`, `Fatal`, `InvariantViolation`. Panics are bugs; corruption is fail-stop.
Secrets use `Secret<T>` and never print their value.

## Determinism rules

Deterministic crates (everything except those in `tools/allow-banned.toml`) must not call real time,
threads, the filesystem or randomness directly. Use the seams in `lattice-seams`. `clippy.toml`
enforces this.

## Versioning

Wire protocol, on-disk formats and LSL grammar have independent versions recorded in `formats/`.
A change needs an RFC, an ADR, a freeze-corpus update and a migration plan. Readers of version N read
all versions up to N and refuse newer ones.

## Commits and review

Conventional Commits, scope is the crate or directory. Trailer `Requirement: P<phase>-<nn>`. Trunk based,
squash merge, protected `main`.

## Definitions

**Ready**: requirement id, acceptance criteria, invariants touched, verification method, no unresolved ADR.
**Done**: merged with green required checks; tests of the declared type; invariants covered; docs
updated; benchmark or eval delta when relevant; backlog item closed; no new `unsafe` outside `lattice-sys`.
