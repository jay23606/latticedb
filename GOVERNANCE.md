# Governance

- **Maintainers** own the repository, review changes, run phase gates and cut releases. A change to
  `formats/`, `lattice-core`, policy code or `clippy.toml` needs two maintainer approvals.
- **Decisions** are made by lazy consensus in pull requests. Cross-cutting decisions use an RFC
  (`docs/rfc`) and are recorded as an ADR (`docs/adr`).
- **Phase gates** follow `docs/spec/README.md`. A gate is signed by two maintainers.
- **Releases** follow the process in `docs/spec/12-ga-and-oss-launch.md`.
- **Becoming a maintainer**: sustained, high-quality contributions and nomination by an existing
  maintainer, approved by lazy consensus among maintainers.

## Policies

- Flaky tests: `docs/policies/flaky-tests.md`.
- Branch protection (settings as code): `docs/policies/branch-protection.md`.
