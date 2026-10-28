# backlog

Working items for the requirements in `docs/spec`, one markdown file per requirement id with front
matter (`id`, `type`, `state`, `requirement`, `phase`, `assignee`, `links`).

The items are generated, not committed. Create them locally with `cargo xtask backlog seed`, edit the
`state` as work proceeds, and validate with `cargo xtask backlog lint`. Once Lattice can host its own
backlog (Phase 6) this directory goes away.
