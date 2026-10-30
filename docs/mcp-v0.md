# MCP server v0

First slice of the Model Context Protocol server (WP52, issue #254, requirement P3-13): a sans-I/O
message codec, three scoped tools and a grant layer, in `crates/lattice-sdk/src/mcp.rs`, and a tool
backend over the SQL executor in `crates/latticed/src/mcp_backend.rs` (issue #526, part 1, see
"Backend over the executor"). The one transport is HTTP: `POST /mcp` on the sync gateway port (issue
#527, part 1, see "HTTP transport"). There is no stdio transport and no streaming transport, and no phase
exit criterion is claimed.

## Protocol revision

The module targets MCP specification revision **2026-07-28**
(<https://modelcontextprotocol.io/specification/2026-07-28>, read on 2026-10-29). That revision has no
handshake: every request must carry `_meta` with `io.modelcontextprotocol/protocolVersion` and
`io.modelcontextprotocol/clientCapabilities`. The server is conformant to that: a request without
them is refused. For clients that still open with the handshake of **2025-11-25**, the server is
dual-era, as the revision allows: a connection that opens with a successful `initialize` is served
with legacy semantics. The server keeps one piece of per-connection state, the mode (`Stateless` at
start, `Legacy` after `initialize`). Only `initialize` sets it; a missing `_meta` never does. Use one
`Server` per connection.

## Supported subset

| Method | Behaviour |
|---|---|
| `initialize` | Legacy. Requires `protocolVersion`, `capabilities`, `clientInfo`; replies with version `2025-11-25`, the `tools` capability and `serverInfo`, and switches the connection to legacy mode. |
| `notifications/initialized` and any other notification | Accepted, never answered. |
| `ping` | Empty `complete` result. |
| `server/discover` | `supportedVersions` (`2026-07-28`; plus `2025-11-25` on a legacy connection), `capabilities.tools`, `serverInfo` in `_meta`. |
| `tools/list` | The tools the caller holds a grant for, in fixed order, each with its `inputSchema`. No pagination. |
| `tools/call` | Named tool and arguments; result is one text content block with `isError`. |
| anything else | `-32601` method not found. |

Every result carries `resultType: "complete"`.

### Per-request `_meta`

| Connection | `_meta` missing | `_meta` not an object | version or capabilities member missing or mistyped | version not accepted |
|---|---|---|---|---|
| Stateless (default `Server::new`) | `-32602` | `-32602` | `-32602` | `-32022` |
| Legacy (after `initialize`, or `Server::sessionless` for the HTTP endpoint) | accepted | `-32602` | capabilities mistyped: `-32602`; absent: accepted | `-32022` |

This applies to `ping`, `server/discover`, `tools/list` and `tools/call`. A stateless connection
accepts only `2026-07-28` (`data.supported` is `["2026-07-28"]`); a legacy connection also accepts
`2025-11-25` (`data.supported` lists both). `data.requested` echoes the requested version only when
it is a short plain string. Unknown methods are `-32601` regardless of `_meta`.

The HTTP endpoint (`POST /mcp`, see "HTTP transport") has no session, so each request is answered in the
Legacy row: `_meta` is optional and `initialize` is not needed first, which is how standard HTTP MCP clients
(Claude Code, Cursor) call it. A `_meta` that is present is still validated, so a version other than the two
revisions is `-32022` on every request.

### Message rules

- Strict JSON-RPC 2.0: `jsonrpc` must be `"2.0"`; the only members are `jsonrpc`, `id`, `method`,
  `params`; `id` is a string or an integer, never null; `params` is an object.
- A JSON array (a batch) is refused with `-32600`: the revision defines no batching.
- A response sent to the server is refused (`-32600`); the server issues no requests of its own.
- Bounds: 256 KiB per message, nesting depth 12, 64 members per array or object, 4096 values per
  message, 65536-character `value`, 1 MiB tool output. Duplicate object keys, lone surrogates, raw control
  characters, leading zeros and a byte order mark are parse errors (`-32700`).
- The JSON reader is local to `mcp.rs`. It is separate from the plugin-contract grammar in
  `contract.rs`, which has no negative numbers, no fractions and a depth of six.

## Tools

All inputs are objects with `additionalProperties: false`.

| Tool | Required | Optional |
|---|---|---|
| `lattice_read` | `scope`, `table` | `key` (omit for a page), `limit` 1..1000 |
| `lattice_write` | `scope`, `table`, `key`, `value` (up to 64 KiB) | `expected_version` |
| `lattice_subscribe` | `scope`, `table` | `after` (feed cursor: 24 lowercase hex digits, the origin when omitted), `wait_ms` 0..30000 (default 0) |

`scope` is a path of `/`-separated segments of `[A-Za-z0-9_-]` (no `.`, no empty segment, at most 128
characters); `table` is `[A-Za-z0-9_]` up to 64 characters. `minLength` and `maxLength` count
characters, as JSON Schema does (the 256 KiB message bound limits the bytes). `key` may contain no
control characters; `value` may contain none except tab, line feed and carriage return. Integer
arguments follow JSON Schema 2020-12: `10`, `10.0` and `1e1` are all the integer 10; `1.5` is a
wrong type. (JSON-RPC ids stay integer or string only.)

Handlers are the `ToolBackend` trait: `call(&mut self, &Caller, &ToolCall)`. The backend receives only
validated, typed calls, and still owns row-level policy (`BackendError::PolicyDenied`).

A backend's output is bounded by its type: `ToolOutput::new` refuses text over 1 MiB, and
`ToolOutput::or_notice` swaps it for a fixed notice. `ToolOutput::tool_error` is the same bounded text
reported as a tool execution error (`isError: true`), for failures the model can correct. The server never fails a call after the backend
returns success, so a write that was applied is never reported as an error that a client would retry.
A write backend must build or bound its output before it commits, or return the notice.
`lattice_subscribe` is a poll with a bounded wait (see "Subscribe" below); streaming notifications are
not implemented.

## Scoping

Every call is made by a `Caller { id, grants }`. A `Grant` pairs a tool with a scope subtree (`a/b`
covers `a/b` and `a/b/c`, not `a/bc`; `*` covers every scope). Order of checks in `tools/call`:

1. tool exists, else `-32602 unknown tool`;
2. caller holds any grant for the tool, else `40301 tool not granted`;
3. arguments valid against the schema, else `-32602` (`data.field` names the schema property at fault);
4. a grant covers the target scope, else `40302 scope denied`;
5. only then the backend is called.

Refusal messages are fixed strings. They never contain caller-supplied names or values or anything
from the store. A backend `PolicyDenied` is also `40302`, `Unavailable` is `50001`, and a version
`Conflict` is a tool execution error (`isError: true`), which the specification reserves for failures
the model can correct. Application codes sit outside `-32768..-32000`, as the specification advises.

## Backend over the executor

`latticed::mcp_backend::McpBackend` implements `ToolBackend` for `lattice_read` and `lattice_write`.
It has no transport of its own: the HTTP endpoint (see "HTTP transport") creates one backend per request
from the principal the bearer token verified, passes `McpBackend::caller()` to `Server::handle` and nothing
else.
Part of the layering decision of issue #526: `layers.toml` lets `latticed` depend on `lattice-sdk`, the
backend lives in `latticed`, and `lattice-api` gained `PrincipalSql` (SQL for one fixed principal).

- **Identity.** The backend is bound to a `Principal` (from the credential check, never from a
  string) and runs every statement as it through the executor actor, with the row policy, author id,
  statement ceiling and memory budget of a PostgreSQL connection of the same account. The `Caller`
  given with each call must equal the one the backend derived from the principal (`caller_for`); any
  other is `40302`. Arguments cannot name an identity: the schemas have no such member.
- **Grants.** A tool grant names a scope, not a table. `caller_for` grants `lattice_read` and
  `lattice_subscribe` on the workspace scope when the principal holds the read bit on at least one table
  (subscribing is reading), `lattice_write` when it holds the write bit on at least one; `allow-all` gets
  all three, `deny-all` nothing. Which tables is the executor's decision on every statement (interim table
  grants, issue #522): a table the principal holds no bit on is invisible.
- **Scope.** The `scope` argument must be the workspace the backend serves; `table` must be `[a-z0-9_]`
  up to 64 characters. `sync_scope` maps the pair to the sync scope `table/<name>` and refuses what it
  cannot express (another scope, or a name the catalog cannot hold), one pair per scope.
- **Rows.** A table is addressable when it has `key text PRIMARY KEY`, `value text` and
  `version bigint NOT NULL` (other columns are returned by `lattice_read`). The version is 1 on
  insert and rises by one on every `lattice_write`; it is not maintained for writes made with SQL, so
  `expected_version` guards MCP writers against each other. `expected_version` 0 means the row must not
  exist. A write is one statement (`UPDATE ... WHERE key = $1 AND version = $3`, or `INSERT`), so the
  check and the write are one commit. A mismatch, or a row that is not at that version, is the tool error
  `version conflict`; the stored row is unchanged. Statements use bound parameters; the table name is
  checked by `sync_scope` before it is quoted into the text.
- **Output.** `lattice_read` returns one JSON text in a fixed envelope:
  `{"tool":"lattice_read","notice":"<NOTICE>","truncated":false,"untrusted":{"rows":[{"key":"k","value":"v","version":"1"}]}}`.
  Everything from storage (keys, column names, values, all as text or `null`) sits under `untrusted`;
  everything outside it is written by the backend, and `notice` says so. Rows are added while they fit
  in 1 MiB less 1 KiB; the first that does not ends the list and sets `truncated`. `lattice_write`
  returns `{"tool":"lattice_write","applied":true,"version":N}` and never repeats the value, so its size
  does not depend on what was written. A page read holds at most `limit` rows (1000) in the executor's
  statement memory budget before it is cut.
- **Errors.** Tool errors have fixed text: `{"error":"not_found"}` (no such table, a table the
  principal may not see, an inexpressible table name or sub-scope, or no such row: one answer for all),
  `permission_denied` (the table is visible but not writable), `table_shape` (not a key/value table)
  and `rejected` (the table refused the value). A stopped executor or an exceeded statement ceiling is
  `50001`. No error holds caller-supplied or stored text.

## Subscribe

`lattice_subscribe` is the poll-with-cursor floor of the live collaboration design (issue #579, part 2 of
#526; `docs/design/live-collaboration.md`): an agent that cannot hold a stream calls it with the cursor it
saved and, with `wait_ms`, is woken when a commit arrives. It reads the change feed as the connection's
principal, so it sees exactly the events a PostgreSQL reader of the same account would get from the feed.

- **Cursor.** The 12-byte feed cursor (commit LSN as a big-endian `u64`, then the index in its change set
  as a big-endian `u32`), written as 24 lowercase hex digits. Only that spelling is accepted; upper case
  or another length is `-32602`. Omitting `after` starts from the origin (24 zeros). Events come back
  strictly after the cursor, in cursor order.
- **Wait.** `wait_ms` is at most 30 000 and defaults to 0, which answers at once. When no readable event
  exists, the call waits on the host's commit notifier for at most `wait_ms`, then answers with no events
  at the cursor it was given. Every wake re-reads the feed; a wake that finds nothing this principal can
  read (a commit to a table it may not see) is not reported, the wait goes on, so a hidden commit does not
  end the call (threat model G-07). Shutdown of the host ends a waiting call with `50001`. The content of a
  reply does not depend on hidden commits; its latency may (see Known limits).
- **Reply.** One JSON text in the fixed envelope:
  `{"tool":"lattice_subscribe","notice":"<NOTICE>","truncated":false,"next_cursor":"<24 hex>","untrusted":{"events":[{"cursor":"<24 hex>","op":"upsert","author":"<32 hex>","entity":"<hex>","columns":{"key":"k","value":"v","version":"1"}}]}}`.
  `op` is `upsert` or `delete`; `author` is the 16-byte author id of the commit (`00..00` when unknown);
  `entity` is the row's raw key in hex; `columns` are the row's columns as text (`null` for SQL `NULL`),
  present for an upsert the executor can decode, and `null` for a delete or for a table recovered from the
  log without its definition. Everything except `tool`, `notice`, `truncated` and `next_cursor` sits under
  `untrusted`.
- **Bounds.** A reply carries at most 64 events (the codec's bound on an array, `MAX_ITEMS`) and at most the
  1 MiB output bound less the envelope. A reply that stops early sets `truncated`; the next call from its
  `next_cursor` continues. An event too large to return with the rest is returned on its own, without its
  columns (`"omitted":"too_large"`), so a reader is never stuck on one event.
- **Cursor movement.** `next_cursor` is the cursor of the last event returned, or the cursor given when none
  was. It moves only on events the principal can read, so the reply does not reveal how many hidden commits
  there were. Reading on from `next_cursor` never skips and never repeats an event.
- **Not returned.** A count of unseen changes. It would tell a table-restricted reader how many hidden
  commits happened (issue #363), so no reply carries it; `truncated` says only that more events exist.
- **A cursor ahead of the newest commit** is not an error. It is answered exactly as a caught-up cursor
  is: no events, `truncated` false, `next_cursor` equal to the cursor given, and the same `wait_ms` wait.
  Once the head reaches it, events after it are returned as usual. Refusing it would tell a reader of any
  table it can read where the head is, and with it how many commits are hidden from it (issue #363).
- **Errors.** `{"error":"invalid_cursor"}`: the cursor is malformed, which is decided from its bytes before
  the store is read (LSN 0 with a nonzero index; the wire-form check already refuses other spellings with
  `-32602`). `{"error":"cursor_too_old"}`: the cursor is below the vacuum floor, so the history it needs may
  be gone; the client must read the table again and subscribe from a new cursor (snapshot-then-tail, issue
  #525, is not built). The floor is a store-wide value, so this error does not depend on what the principal
  may read. A table the principal may not read is `{"error":"not_found"}`, the same answer as a table that
  does not exist.
- **Known limits (issue #363 is not closed).** Each of these is a signal a table-restricted reader can
  observe. None is a count of hidden commits in the reply itself.
  - Quiet-table wedge: the feed scan is not per table. A reader of a quiet table can meet a large run of
    commits to other tables, and once that run passes the step budget of `feed_source` (about 4.2 million
    versions by default) every poll answers `50001`. The threshold reveals that hidden activity was large
    (issues #525 and #661 are the mitigations).
  - Timing: a visible event answers at once. Each wake re-reads the feed, so a waiting call's latency
    rises with hidden activity, while its reply stays the same.
  - A corrupt event: if a feed event's bytes do not decode, the whole call fails with `50001` and every
    later call from the same cursor fails the same way. Only corrupted feed bytes can cause this; it needs
    the store repaired or reopened.
  - Long-poll budget: there is no per-principal cap on waiting or feed work (issues #529 and #543). A
    waiting call holds its worker for up to 30 seconds. When the notifier has no room for another sleeper
    (the host's commit-waiter limit), the call polls the feed about 50 times a second until the cap of
    #529 bounds the waiters.

## HTTP transport

`POST /mcp` on the sync gateway port (`latticed --data-dir DIR --gateway-listen 127.0.0.1:PORT
--gateway-tokens FILE`; issue #527, part 1, with the browser and DNS-rebinding defences of issue #580). The
gateway is loopback only and has no TLS yet (issue #578), so a non-loopback address is refused at start.
Tokens come from the credential file given to `--gateway-tokens` (the same file the WebSocket gateway reads, re-read
within 5 seconds of a change). The development gateway (`development-trust`, without `--gateway-tokens`) has no
credential store, so `POST /mcp` answers `401` to every request there. Those refusals are not counted against
the address, as the WebSocket upgrade on that gateway is not, so they never become `429`.

Scope of this part. It is part 1 of #527 (the POST transport and bearer authentication) together with the
browser defences that are part 2 (#580: Host, Origin, content type, CORS, the declared length), which ship
together so that no release has the endpoint without them. Not in it: the long-poll budget (#529). A
`lattice_subscribe` call with `wait_ms` holds a connection for up to 30 seconds and counts only against the
gateway's connection limit.

Plaintext. The bearer token travels in the `Authorization` header unencrypted over loopback until TLS on the
gateway port exists (issue #578). The listener refuses any non-loopback address, so the token does not leave
the host; do not forward the port or put it behind a proxy.

A request is one MCP JSON-RPC message in the body. Its headers must be:

- `Host`: this listener's address (`127.0.0.1:PORT`) or `localhost:PORT`, exactly once. Anything else is `403`.
- `Authorization: Bearer <token>`: the same verification, failure count and rate limit as the WebSocket
  upgrade. A missing, malformed, unknown or revoked token gets the same `401` bytes. Too many failures from
  one address get `429`.
- `Content-Type: application/json`, optionally `; charset=utf-8`. Anything else is `415`.
- `Content-Length`, once. A missing length or any `Transfer-Encoding` is `411`; a non-numeric one is `400`.
  A length over 1 MiB is `413` before any body byte is read.

The request must not carry an `Origin` header (any value, `null` included: `403`). No response carries a CORS
header, so a page on another origin cannot call a tool, and a request with a query string is `400` (a token
is never accepted in a URL and is never echoed).

| Request | Response |
|---|---|
| `POST /mcp`, valid | `200`, `Content-Type: application/json`, one JSON-RPC response |
| a notification (no `id`) | `202` with an empty body |
| malformed JSON | `200` with a JSON-RPC parse error (`-32700`) |
| any method other than `POST` | `405`, `Allow: POST` |
| a target other than `/mcp` on this port | the WebSocket rules: `404` for an unknown path, `400` for a non-`GET` request |
| more than 64 header lines, or a head over the gateway's head limit (8 KiB by default) | `431` |

Every response says `Connection: close` and `Cache-Control: no-store`; there is no keep-alive. The head and
body are read under the gateway's handshake deadline (default 5 seconds) and a body that stops short is cut
at the deadline without an answer. The principal is the one the token verified and nothing in the body
changes it: `_meta` cannot name another identity (a body that names one is answered as the bound principal).

Sessionless. Each request is served by its own `mcp::Server::sessionless`, and nothing carries from one request
to the next: no session id is issued or required. `initialize` is answered with `2025-11-25` and changes nothing
else; `tools/list` and `tools/call` work with no prior `initialize` and with or without `_meta`. A `_meta` that
is present is validated (see "Per-request `_meta`"). Standard HTTP MCP clients (Claude Code, Cursor) send
`initialize`, `notifications/initialized`, `tools/list` and `tools/call` with no `_meta` and no session id, and
those four requests succeed as shown by `a_standard_client_with_no_meta_and_no_session_can_call_tools`.
One manual run with the Claude Code CLI 2.1.314 (`claude -p` with an `--mcp-config` of type `http` whose headers
carry the bearer token, `--strict-mcp-config`, and the two tools allowed) wrote a row with `lattice_write` and read
it back with `lattice_read`; the row is at version 1 in the table. This is one manual check, not a test.

Ordering of the deadlines: the head is read under the handshake deadline. The handshake place is released once
the token verifies; the body is then read under the same deadline. A client that authenticates and stops short
therefore holds a live-connection place only until that deadline (default 5 seconds) passes, and no longer.

A `401` on `/mcp` is byte-identical across missing, malformed, unknown and revoked tokens, which is the same
answer the WebSocket upgrade gives (status, `WWW-Authenticate: Bearer` and the body `authentication required`).
The one difference is the `Cache-Control: no-store` header that every `/mcp` response carries. The WebSocket 401
does not carry it.

The planted defects of this part (`McpHostCheckSkipped`, `McpOriginCheckSkipped`, `McpBodyCheckedAfterRead`,
`McpPrincipalFromBody`) are switched by a thread-local mask, so their witnesses are pure-function witnesses
(`ws::mcp_http`) and the socket tests cannot switch them on. `McpPrincipalFromBody` is also masked by the
backend: `McpBackend` refuses a call whose `Caller` is not the one derived from its principal (`PolicyDenied`,
`40302`), so that defect alone does not widen access; the witness is what detects it.

The tool `scope` must be the gateway's MCP workspace, `default` (the default of `GatewayConfig::mcp_workspace`;
the CLI does not set it yet). The `Accept` header is not checked: every answer is JSON.

Not here: stdio, Streamable HTTP streams (SSE, GET, sessions and resumption), TLS, and the per-principal limit
on long polls (issue #529). A `lattice_subscribe` call with `wait_ms` holds its connection thread for up to
30 seconds; the connection counts against the gateway's connection limit, not a per-principal budget.

## Not implemented

- Transports other than HTTP `POST /mcp`: stdio framing, and Streamable HTTP streams (SSE, GET, sessions).
- Authentication and authorization framework: the transport supplies the `Caller`; nothing here
  verifies identity or issues grants.
- Resources, prompts, completion, logging, sampling, elicitation and roots.
- Tasks, multi round-trip requests (`input_required`), progress, cancellation, caching hints and
  pagination cursors.
- `subscriptions/listen` and `notifications/tools/list_changed`.
- Output schemas, structured content and tool annotations.
- Rate limiting.
- A cursor in `lattice_read` and `lattice_write` results. The cursor type now exists (see "Subscribe"),
  but no read or write result carries one yet.
- A count of unseen changes. It is not built, and would reveal hidden commits to a restricted reader
  (issue #363), so it is not planned for a restricted caller.
- The per-principal limit on long polls (issue #529): a `wait_ms` call holds a connection's worker for up
  to 30 seconds, and only the host's commit-waiter limit bounds how many sleep at once.
- Presence, search, document and workflow tools.

## Evidence

The executor backend's tests (`cargo test -p latticed --lib mcp_backend`) run the server over a real
`ExecHost` with bearer-authenticated principals; they check writes against a SQL reader and the change
feed, version conflicts (also from concurrent writers), the fixed envelope byte for byte, denied equal to
missing, the output bound, the scope mapping and the grant mapping. Five planted bugs are each caught by a
witness: `McpCallerFromArgument`, `McpWriteIgnoresExpectedVersion`, `McpDeniedTableDistinguishable`,
`McpRowTextOutsideEnvelope` and `McpSubscribeInclusiveCursor`.

The subscribe tests in the same module check the reply against the change feed read directly (the same
cursors and authors, in order), a seeded interleaving of writes and calls that must yield every key once in
write order, the wait bound and wake-up, shutdown cancelling a waiting call, two principals with hidden
commits that do not move the cursor, a delete, the event and byte bounds with resumption, a cursor ahead of
the head answered as caught up (identically for two principals, with and without hidden commits, and equal
to the reply at the head apart from the echoed cursor), and refusal of a malformed cursor.

Unit and property tests live beside the code (`cargo test -p lattice-sdk mcp`): byte vectors per
message, a schema table, a grant table, a `_meta` table, a dual-era connection test, a mock backend that
records calls to prove it is never reached on denial or invalid input (and that an oversize write
output is applied once and reported as success), arbitrary-bytes and valid-request properties, and an
injection test.
The fuzz target is `fuzz/fuzz_targets/mcp_message.rs` with seeds in `fuzz/seeds/mcp_message/`; it is
not yet in a scheduled fuzz matrix.
