# ADR 0033: Chaining the SQL board and the MCP feed

Status: proposed

Issue #756 (part of the alpha 0.2 dogfooding milestone). This ADR is proposed. It becomes a decision only when the owner or the tech lead accepts it; the existence of the pull request is not acceptance. It changes no wire format, no stored format, no board command and no gate, and it claims no measured cost.

## Context

Two cursors answer "what changed since I last looked", and nothing translates one into the other.

- **Board event ids.** The agent board (`integrations/board/board`, conventions in `integrations/board/CONVENTIONS.md`) numbers its events from a counter row. `board head`, `board since` and `board watch` take and print these ids. The ids are `bigint` (`integrations/board/schema.sql:38-39`) and the schema says they "rise in commit order" (`schema.sql:34-36`). Each write command is one transaction (`integrations/board/board:85`, `BEGIN` at line 166).
- **Feed cursors.** `lattice_subscribe` reads the change feed. Its cursor is 12 bytes, the commit LSN as a big-endian `u64` followed by the index in the change set as a big-endian `u32`, written as 24 lowercase hex digits (`docs/mcp-v0.md:173-175`, `crates/lattice-sdk/src/mcp.rs:60`). The call takes `scope` and `table` and an optional `after`, and returns `next_cursor` (`docs/mcp-v0.md:74`). The feed encoding is an experimental candidate, not frozen (`docs/glossary.md:21-25`, ADR 0023, proposed).

`integrations/board/CONVENTIONS.md:59-65` already says the two are separate and that `board head` prints no feed cursor, because `docs/sql-subset.md` has no way to read the feed from SQL (checked: that file has no mention of the feed or of an LSN). The second request in #756 is already done on main: the `after` argument and the tool description say the cursor is optional, is the previous `next_cursor`, and is not a board id (`crates/lattice-sdk/src/mcp.rs:750`, `:787`). A worker that passes a board id as `after` gets a schema error, not a silent wrong answer, because `after` must be exactly 24 characters (`mcp.rs:742-752`).

What is still missing is a way to get from one cursor to the other. Three facts constrain the options.

1. **A feed cursor exists only after the commit.** The LSN is attached when a change set is logged (`crates/lattice-core/src/txn.rs:203`, `Txn::logged(cs, lsn)`), and the caller learns it from `Receipt::Recorded { lsn, hlc }` (`crates/lattice-core/src/kernel.rs:50-56`, `:368-372`). The rows of a transaction are fixed before that, so a board row cannot store the feed cursor of its own commit. (This is a reading of those call sites, not a test.)
2. **The feed is per table and read as the principal.** Subscribe maps `(scope, table)` to the sync scope `table/<name>` (`crates/latticed/src/mcp_backend.rs:105`, `:134-147`) and reads the feed as the bound principal (`docs/mcp-v0.md:170-171`, `crates/latticed/src/mcp_backend.rs:647`, `crates/lattice-api/src/feed_source.rs:177`). The module documents a `key`/`value`/`version` layout for `lattice_read` and `lattice_write` (`mcp_backend.rs:45-49`), and those two arms call `table_of` (lines 254 and 275). The subscribe arm does not (`mcp_backend.rs:629-660`), so the board's `events` table is not excluded by that layout. This is a reading of the code, not a test.
3. **A cursor must not reveal the head.** A cursor ahead of the newest commit is answered as caught up, not refused, because refusing it would reveal the newest LSN and with it the number of commits hidden from the reader (`docs/mcp-v0.md:200-202`, `crates/latticed/src/mcp_backend.rs:624-626`, issue #363). Issue #363 is still open (`docs/alpha.md:296`).

### Options

**(a) `board head` also prints the feed cursor.** Not chosen. There is no SQL path to the feed or to an LSN, so the command would need new server support. It also changes a documented contract: `board head` "prints the newest board event id ... and nothing else" (`CONVENTIONS.md:57`), and the convention loop starts with `CURSOR=$(board head)` (`CONVENTIONS.md:107`). A feed head printed to any SQL reader would expose the newest LSN to readers who cannot read the table, which fact 3 forbids.

**(b) A `changes_since` MCP tool that takes either cursor.** Deferred. It adds a tool to the MCP surface, which `docs/mcp-v0.md` defines and `Tool::ALL` lists (`crates/lattice-sdk/src/mcp.rs:764`). A wire change needs an RFC, an ADR, a freeze-corpus update and a migration plan (`docs/conventions.md:33-34`), so it cannot land with this ADR. The two inputs are also distinguishable by length: a board id is a `bigint` (at most 19 decimal digits) and a feed cursor is always 24 characters. But the server still needs the board id to feed cursor map. Fact 1 means the map cannot be written in the board's own transaction; it could only be built by scanning the feed for the board table (the same work as option c) or written in a second transaction, which is not atomic with the event.

**(c) Keep two cursors and document the translation.** Proposed. No code. A worker that holds board id `N` and wants feed events after it:

1. calls `lattice_subscribe` on table `events` from the origin (omitting `after`), following `next_cursor`, until it finds the event whose `id` column is `N`;
2. keeps that event's `cursor` as the feed position, and passes it as `after` from then on.

The reverse direction reads the `id` column of the last event returned and passes it to `board since`. Both directions rely on the decoded `columns` of an upsert (`docs/mcp-v0.md:186-190`), which the reply carries only when the executor can decode the row. Nothing here has been run. The open questions below list what must be checked first.

## Decision

1. Adopt option (c) as the working design: board event ids stay the board's cursor, `lattice_subscribe` keeps the feed cursor, and neither command or tool changes. No wire, format, or stored-data change follows from this ADR.
2. Before the translation is written into `integrations/board/CONVENTIONS.md`, check open questions 1 to 4 with a recorded run or a test, and put the result in that pull request. Until then the convention keeps saying the two are separate and gives no translation.
3. Do not add a feed cursor to `board head` (option a). Do not add a `changes_since` tool (option b) without an RFC. Revisit (b) only if a measured scan cost makes option (c) unusable for the board's size.
4. Any future surface that returns, accepts or compares feed cursors must not reveal the newest LSN, or any hidden commit, to a reader without the grant on that table (`mcp_backend.rs:624-626`, issue #363).
5. A cursor is a position, not a capability. Each call keeps its own checks: the caller must equal the one derived from the principal (`mcp_backend.rs:577-586`), and the feed is read as that principal.

## Consequences

- Nothing changes for current users. `board head`, `since`, `watch` and `lattice_subscribe` behave as they do on main, and `docs/mcp-v0.md` is not edited.
- The worker problem in #756 (a reader that starts from the origin because it cannot translate) stays until the translation is documented and checked. Option (c) turns that scan into a documented, bounded procedure rather than an accident.
- The feed encoding is not frozen (`docs/glossary.md:23-25`, ADR 0023). The translation must not promise a cursor's bytes stay stable across releases. A recipe in the board convention must say so.
- Cost of (c): a scan from the origin reads at most 64 events per call (`mcp_backend.rs:97`, `docs/mcp-v0.md:191-192`), so reaching id `N` takes about `N / 64` calls, plus any wait. A reader past the vacuum floor gets `cursor_too_old` and must start again (`docs/mcp-v0.md:204-208`). Nothing measured here shows what this costs at the board's size.
- Security: the board's ids are a count of events and already reveal how many exist to anyone who can read the table (`schema.sql:34-36`). This ADR does not change that. A feed cursor copied to another principal yields only what that principal may read.
- The feed scan is store-wide. A reader of a quiet table can be stalled by heavy traffic on other tables (`docs/mcp-v0.md`, "Known limits"; `docs/alpha.md:297`, issue #543). A board scan on `events` could hit this.

### Open questions (not verified by this ADR)

1. Does `lattice_subscribe` on table `events` return the board's rows, with `columns` that include `id`? Subscribe does not check the table shape (see fact 2), but the executor's decoding of a `bigint` key has not been run here.
2. Does the principal the board runs as hold a read grant on `events`? Grants are per table and interim (issue #522, `mcp_backend.rs:21-27`).
3. Do feed cursors order the same way as board ids when two board writers commit at the same moment? The schema states it (`schema.sql:34-36`); this ADR has not tested it. Each board write is one transaction (`board:85`), so one command should map to one cursor, which is also untested.
4. How far back does the feed for `events` reach before `cursor_too_old`, and how long does a full scan take at realistic board sizes? These need a measurement with a recorded number, not an estimate.
5. Should a board-facing recipe name the feed cursor's bytes at all, given the encoding is not frozen (ADR 0023)? Owner decision.
