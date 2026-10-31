# lattice-cli

Layer L7.

**Responsibility.** Admin and scripting command line tool.

**Must not.** Contain server logic.

Allowed workspace dependencies: lattice-abi, lattice-sdk, lattice-core, lattice-seams, lattice-lsl,
lattice-api, latticed.

`lattice account init|add|list|remove|token create|token revoke|token list` provisions the SCRAM
verifier file, the decoy secret, the per-table policy file and the bearer-token file that `latticed`
reads, with owner-only files and secrets from stdin or generated and shown once. It reuses the file
rules of `latticed::files` and the verifier and token code of `lattice-api`. See
[the listener guide](../latticed/README.md#provisioning-with-lattice-account) and
`lattice account --help`.

`lattice watch table/NAME --url ws://127.0.0.1:PORT --token-file FILE (--from CURSOR | --from-start | --cursor-file FILE)`
connects to the sync gateway on a loopback address, subscribes to the table and prints one line per
event: `CURSOR AUTHOR TABLE KEY OPERATION`, with `CURSOR` as 24 lowercase hex digits, the form `--from`
takes. Each event is printed, then acknowledged, then its cursor is saved, so a crash re-prints an
event on the next start and never skips one. The URL must be `ws://` on a loopback address with a
port (the client has no TLS yet). The token comes from a file, which must be owner-only like the other
credential files (a regular file, not a symlink, mode 0600 or stricter), or from standard input with
`-`; a token on the command line is refused.

`--cursor-file FILE` keeps the saved cursor in FILE. The directory of FILE must exist and must not be
writable by group or others; FILE must be owner-only (mode 0600) and is replaced atomically (temporary
file, sync, rename). A missing file starts at the beginning of the table; a file that does not hold
exactly one cursor is refused and left as it is. Two watches on one cursor file are refused: the watch
holds `FILE.lock` for its whole run, and a second one exits at once. Without the flag the watch starts
from `--from` or `--from-start` and keeps no cursor on disk.

A dropped connection is retried from the last saved cursor. The wait before a retry starts at one
second and doubles with each failure, up to eight seconds. A connection that delivers no event counts
as a failure, and three failures in a row stop the watch with status 1; a connection that delivers an
event starts the count again. A refused token (HTTP 401 or 403) and a close with code 1008 (policy)
stop the watch at once, because a new connection would be refused the same way. The retry wait is
local: the SDK's jittered backoff (#778) is not in the tree yet. SIGTERM and SIGINT stop the watch after
the event in hand is saved, with status 0. Not done: TLS on the client, and an end-to-end test against
a live gateway (the e2e harness cannot run this binary, see #583).
`lattice watch --help` lists the flags.

`lattice mcp --url http://127.0.0.1:PORT/mcp --token-file FILE` is a stdio proxy to the gateway's
`POST /mcp` endpoint (issue #775). It reads one JSON-RPC message per line from standard input and writes
each reply as one line on standard output; diagnostics go to standard error. The URL must be `http://` on
a loopback address with a port and the path `/mcp` (the CLI has no TLS client yet). The token is read from
an owner-only file, or from the first line of standard input with `-`, and a token on the command line is
refused. A message over 1 MiB is refused with a JSON-RPC error. `lattice mcp --help` lists the flags.
