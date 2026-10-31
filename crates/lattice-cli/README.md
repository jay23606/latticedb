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

`lattice watch table/NAME --url ws://127.0.0.1:PORT --token-file FILE (--from CURSOR | --from-start)`
checks its arguments today. The URL must be `ws://` on a loopback address with a port (the client has
no TLS yet). The token comes from a file, which must be owner-only like the other credential files
(a regular file, not a symlink, mode 0600 or stricter), or from standard input with `-`. A token on
the command line is refused. The event formatter (`CURSOR AUTHOR TABLE KEY OPERATION`, with `CURSOR`
as 24 lowercase hex digits, the form `--from` takes) and the rule that an event is acknowledged only
after its line is printed exist in this build. The network loop that connects and prints events is
not wired yet, so a valid invocation stops with an error saying the transport is not built.
`lattice watch --help` lists the flags.
