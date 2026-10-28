# Phase 0 gate record

Date: 2026-10-28. Evidence: `cargo xtask ci` passes locally on macOS (aarch64) with cargo-deny.

## External review

An independent, read-only review found 29 issues. Applied: determinism lint bans extended
(time, sleep, env, net, process, RandomState); a real Rust lexer for the unsafe and banned-lint checks;
`check-allow` rejects every suppression form; `dev_allow` limited to layer 2 and above; xtask and
lattice-sys now carry the shared lint discipline; feature `intel` (off by default) and engine features;
`cargo deny` and commit-message, conventions, link, config-format, quarantine, RFC and ADR-index checks;
strict backlog lint; SHA-pinned workflows with dependabot; release dry run with SBOM, changelog and
keyless signing steps; nightly and weekly scaffold jobs; devcontainer and hooks; trivial TLA+ and fuzz
targets; docs build includes conventions and ADRs; unknown xtask commands now fail.

## Not done, with reason

| Item | Reason |
|---|---|
| Live branch protection and two-approval rules | Needs a remote repository; settings are provided as code in `tools/branch-protection.json` |
| Working security mailbox | Needs an owner and a domain; SECURITY.md points to GitHub private advisories |
| Release workflow, fuzz job, TLC job, secret scan | Written but unexecuted locally (need GitHub runners, nightly toolchain, Java) |
| Transitive-dependency layer check | Only workspace crates may depend on workspace crates in practice; revisit if external helper crates appear |
| tokio and rand bans in clippy.toml | Added in Phase 1 when those dependencies enter the workspace |
| E0-2 demonstration pull requests | Needs a remote; the same rules are covered by unit tests and a local probe |
| E0-9 newcomer session | Needs a person who did not set the repo up |
