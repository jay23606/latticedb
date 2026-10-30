# ADR 0029: Assign author ids to accounts from a local registry file

Status: proposed

## Context

Issue #520 (part of #518) records who committed a row change. The change set already has a 16 byte
`author` field (`formats/changeset-v1.md`) and the change feed repeats it, so no WAL, change set or feed
byte changes. What is missing is an id per authenticated account that is injective: clients use the
author to ignore their own echoes, and the CRDT node binding of the document work (#532) uses it as an
identity. The credential file has no numeric ids (ADR 0026), and a hash of the account name cannot be
injective for names of up to 255 bytes. `lattice-identity` (Phase 4) will own durable user ids; this ADR
is the interim.

## Decision

`lattice_engines::authors::AuthorRegistry` assigns ids from a counter and keeps the assignment in a local
sidecar file, `authors.reg`, in the data directory beside the log. It is not part of the WAL, the change
set or the feed. The format is experimental and not frozen.

- **Id space.** 0 is unknown or legacy. 1 is the local-development principal. 2..=255 are reserved.
  Accounts take 256 and up, in order of first use, from a counter that only grows.
- **File.** UTF-8 text. First line `lattice-authors v1`. Then one line per change, each ending in a newline:
  `A<TAB>id<TAB>name` (assign), `R<TAB>id<TAB>new name` (rename, id kept), `D<TAB>id` (retire).
  Names are 1 to 255 bytes with no control character, compared byte for byte (case sensitive, no Unicode
  normalisation, so look-alike names are different accounts). An `A` id must be above every earlier id.
  The file is append-only and never rewritten.
- **Durability.** A line is appended and synced (and the directory on creation) before its id is used. A
  failed append is cut back so a partial line cannot glue to the next one.
- **Torn line.** A last line without its newline is a torn append: it is removed at open and its id was never
  used. Any other malformed line, a repeated live name, a repeated or decreasing id, an operation on an id
  never assigned, or a file over 8 MiB fails the open with the line number, never the content.
- **Log cross-check.** After the database opens, the host compares the registry with the account author ids
  in the retained log (`RowStore::account_authors`). A log id the registry does not list, or one at or above
  its counter, refuses startup (`the author registry is missing or older than the log`). This catches a
  deleted file, a restore of an older backup, and a crash that left a synced commit whose registry line was
  not durable. Residual: ids whose commits vacuum already removed from the retained log cannot be checked, so
  a registry lost after such a vacuum can restart the counter at an id that only vacuumed history carries.
- **Bounds.** At most 20,000 ids ever assigned (live and retired), 8 MiB of file. Beyond the id bound a new
  account gets an error on its first statement.
- **Permissions.** Created empty (the `Io` seam takes no mode) and narrowed to 0600 at once, inside a 0700 directory. A symbolic link in its place is refused. An existing file with group or other
  bits and any content is refused at start (Unix).
- **Backup and restore.** Back up `authors.reg` together with the log, taken at the same point, and restore
  both. Restoring an older registry beside a newer log refuses startup rather than reissuing ids. Never edit
  the file by hand.
- **Rename and retire** are library calls (`rename`, `retire`). Nothing calls them today. Removing an account
  from the credential file or the policy file does not retire it, and adding the same name back is the same
  account with the same id, not a new one. A retired id is never reissued, and a retired name used again is a
  new account with a new id. An account that is in the credential store but not in the policy file still gets
  an id on its first statement (it is then denied everything); the number of ids is bounded by the number of
  credentials.
- **Commits with no author.** Commits that bypass the executor carry author 0: `RowStore::commit_catalog`
  (it begins with `begin()`) and the standalone catalog store in `catalog.rs`. History from before this ADR
  is also 0.

## Consequences

- Events say who wrote, ids are injective per account for as long as the registry and the log stay together,
  and the executor needs no client input for it.
- A host that loses the registry together with the log has a fresh history and a fresh registry, which is
  consistent. A host that loses only the registry will not start until an operator restores it.
- The file is local and unauthenticated: whoever can write the data directory can edit it, as they can the
  log. The author is a label, not a signature (Phase 4 audit chain).
- The format needs an independent review before this ADR is accepted, and a migration into
  `lattice-identity` when that crate assigns durable ids.
