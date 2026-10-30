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
