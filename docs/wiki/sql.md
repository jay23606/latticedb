# SQL subset

Lattice speaks PostgreSQL wire protocol v3 and accepts a subset of PostgreSQL's SQL. This page says what the
server does today. The design contract, with its rules and the list of differences from PostgreSQL, is the
[SQL subset document](../sql-subset.md). That document's status line was written before the executor
existed, so for what runs now, use this page.

Results are text format only. Binary result formats are not supported.

## Accepted today

These statements run against a server started with `--data-dir`, on single tables:

- `SELECT`, `INSERT`, `UPDATE`, `DELETE`
- `CREATE TABLE`, `DROP TABLE`
- Transaction control

Column types are the ones listed in the Types section of the [SQL subset document](../sql-subset.md#types):
`boolean`, the integer types, `double precision`, `numeric(p, s)` up to 38 digits, `text`, `varchar(n)`,
`bytea`, `timestamptz`, `date`, `uuid` and `jsonb`. Arrays, ranges, `interval`, geometric types, `inet`,
`money` and `xml` are not supported.

Tables and their column definitions (types, `NOT NULL`, primary key, `UNIQUE`, defaults) are recovered from the
log after a restart. A data directory written by an older build refuses its first `CREATE TABLE` (SQLSTATE
`55000`) until the server starts with `--allow-format-upgrade on`.

## Refused today

Each of these answers `0A000` (feature not supported) rather than returning a wrong answer:

- Joins, `GROUP BY`, aggregates, `DISTINCT` and subqueries
- Quoted identifiers with a capital letter, such as `"Id"`. The stored table definition keeps no ASCII capitals.
- Identifiers longer than 63 bytes (SQLSTATE `54001`)
- `ALTER TABLE`, which is not implemented
- Binary result formats

Some other differences from PostgreSQL are deliberate and listed in the subset document. For example, a
`CAST` to `numeric(p, s)` or `varchar(n)` is refused because plans do not carry type modifiers yet, and
`CREATE TABLE t ()` is refused with `42601`.

## Check it yourself

```sql
CREATE TABLE notes (id INT PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN);
INSERT INTO notes VALUES (1, 'write the spec', false);
INSERT INTO notes VALUES (1, 'again', true);   -- refused: duplicate key (23505)
SELECT * FROM notes;
```

The quickstart in the [README](../../README.md#try-it) runs this kind of script on every pull request that
touches the server crates.

## How correctness is checked

Where a construct is accepted, the target is PostgreSQL's result. The repository runs sqllogictest scripts
from `crates/lattice-testkit/tests/slt`, and a reference script compares the results with PostgreSQL. The
nightly workflow runs that check. The [verification](verification.md) page lists what the evidence covers.
