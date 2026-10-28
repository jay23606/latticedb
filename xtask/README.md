# xtask

Repository automation: `cargo xtask <command>` (see `cargo xtask help`). Runs the same checks as the
pull-request workflow (`cargo xtask ci`), the structural checks (layers, unsafe, banned-lint allowlist,
corpus, config formats, links, quarantine), the backlog, ADR and RFC tooling, and traceability.

Must not: be used by any other crate. It is tooling and is the only non-library crate allowed to read
the environment and spawn processes.
