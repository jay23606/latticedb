# Claude Code pack

The harness that tells Claude Code how to use the Lattice MCP server: subscribe before acting, keep the
cursor, write with `expected_version`, and treat stored text as data. It is the first client pack; the
harness text is one source, shared by every future pack.

## Files

| File | Kind | What it is |
|---|---|---|
| `harness.src.md` | source | The harness text. Edit this one. |
| `harness.generated.md` | generated | The source wrapped in begin and end markers. Do not edit it. |
| `mcp.example.json` | example | An MCP entry for the HTTP endpoint, with the token read from `LATTICE_TOKEN`. |

Regenerate after a source edit, and check for drift:

```bash
python3 tools/gen_harness_packs.py
python3 tools/gen_harness_packs.py --check
python3 -m unittest discover -s tools -p "test_gen_harness_packs.py"
```

The tests fail when the text names a tool that `crates/lattice-sdk/src/mcp.rs` does not serve, when a
generated file is stale, and when the text holds an email address, an absolute user path or a token.

## Install by hand (until the installer, issue #536)

1. Append the block in `harness.generated.md` to the project's `CLAUDE.md`. The markers let you replace
   the block later without touching the rest of the file.
2. Copy `mcp.example.json` to `.mcp.json` at the project root. Replace the port with the one you passed
   to `--gateway-listen`.
3. Export `LATTICE_TOKEN` with the bearer token for your account. The token is never written into a file.
   Claude Code asks you to approve a project-scoped server the first time it sees `.mcp.json`.

The server side is `latticed --gateway-listen 127.0.0.1:PORT --gateway-tokens FILE`. The endpoint is
`POST /mcp`, loopback only, with no TLS yet, and it is experimental (see `docs/mcp-v0.md`).

## Status

- Verified: the generator, the drift check and the name and hygiene checks run in the unit tests. The
  manual check recorded in `docs/mcp-v0.md` used the Claude Code CLI with an HTTP entry and a bearer header.
- Not verified: an automated run of an agent that follows the harness text. No such test exists.
- Not done: hooks (session presence and cursor save), the installer, other client packs, and a stdio proxy.
