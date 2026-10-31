# lattice-e2e

Test support for end-to-end scenarios. `Fixture::start` runs the sync gateway and the SQL listener of
`latticed` in process, on loopback sockets, over a temporary data directory. Two principals hold bearer
tokens: `alice` (every table) and `bob` (read-only on `notes` and `kv`). `Ws` is a raw sync client and `Pg` a
minimal SQL client. `Mcp` is a raw `POST /mcp` client for the MCP tools; `Json` reads the replies.
`kill_points(seed, n)` gives a reproducible kill schedule; the harness does not inject kills yet.

`Fixture::create_kv` makes the key/value table the MCP tools address. `tests/mcp.rs` covers two principals
through the gateway (write, read, grants, conflict, scope and 401) and a cursor chain over a seeded history
(`lattice_subscribe` pages from `next_cursor`, no gap and no repeat). Expected values come from
`docs/mcp-v0.md`.

The crate is not published and has no runtime behaviour. Run it with `cargo test -p lattice-e2e`.
