# lattice-e2e

Test support for end-to-end scenarios. `Fixture::start` runs the sync gateway and the SQL listener of
`latticed` in process, on loopback sockets, over a temporary data directory. Two principals hold bearer
tokens: `alice` (every table) and `bob` (read-only on `notes`). `Ws` is a raw sync client and `Pg` a
minimal SQL client. `kill_points(seed, n)` gives a reproducible kill schedule; the harness does not inject
kills yet.

The crate is not published and has no runtime behaviour. Run it with `cargo test -p lattice-e2e`.
