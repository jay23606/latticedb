# Lattice alpha guide

Status: alpha pre-release, test data only. No phase gate is claimed, no phase exit criterion is met,
and nothing here is production ready. The definition and release checklist are tracked in
[#567](https://github.com/moncifem/latticedb/issues/567) and
[#568](https://github.com/moncifem/latticedb/issues/568).

This page says what works on `main` today, what is experimental, what is missing, and how to report a
problem. It is checked against the code and the gate pages, and where an item lives in an open pull
request or is still missing it says so. If this page and the repository disagree, the repository wins; please open an issue.

## What the alpha is

A PostgreSQL-compatible server (`latticed`) over a crash-safe write-ahead log, with a single-table SQL
subset. You can start it, connect with `psql`, create tables, and insert, select, update and delete rows.
That is all the quickstart claims, and it is exactly what the quickstart smoke test
([`quickstart-smoke.yml`](../.github/workflows/quickstart-smoke.yml)) runs on pull requests that touch the README or the server crates: build
`latticed`, start it, run the README SQL through `psql`, stop it, start it again on the same data
directory and check the documented restart behaviour below.

Platforms: macOS arm64, Linux x86_64, Linux arm64. Not Windows. The Linux archives are built on Ubuntu
22.04 and smoke-tested on Debian 12 and Ubuntu 22.04; the measured minimum glibc is printed in the
release notes ([#588](https://github.com/moncifem/latticedb/issues/588)).

## Install

From source (what the smoke test runs): install Rust with `rustup`, which picks the toolchain pinned in
`rust-toolchain.toml`, then follow [Try it in the README](../README.md#try-it).

From an archive, once the alpha tag exists (not yet verified against a real published release): see
[Install an archive](../README.md#install-an-archive-instead-once-the-alpha-tag-exists) and
[releasing](releasing.md#installing-a-release). The script verifies checksums and, with `gh`, the build
attestation. It is not a release asset yet, so fetch it from the tag as shown there.

## Run `latticed`

Loopback development trust, the mode the quickstart uses:

```bash
latticed --data-dir ./data --development-policy allow-all
```

It listens on `127.0.0.1:5432`, trusts every connection and refuses a non-loopback bind in this mode.
`--development-policy allow-all` is required and never implied. Stop it with Ctrl-C or SIGTERM, or by
typing `shutdown` on stdin. When it runs detached (a container, systemd, launchd) add
`--stdin-control off`, otherwise end-of-file on stdin stops it; the
[listener guide](../crates/latticed/README.md#running-as-a-service) has the exit codes and the service
unit snippets (the unit and plist have not been run under those supervisors).

SCRAM-SHA-256 over TLS, for anything reachable from another host:

```bash
latticed --auth scram-tls --data-dir ./data --account-policies policies.txt \
  --tls-cert server-chain.pem --tls-key server-key.pem \
  --scram-verifiers accounts.txt --scram-decoy-secret decoy-secret.bin
```

Plaintext never carries a password, and the server refuses to start in this mode without TLS. Clients
must verify the certificate (libpq `sslmode=verify-full`); channel binding is not implemented
([#360](https://github.com/moncifem/latticedb/issues/360)). The key and decoy-secret files must be mode
0600. `policies.txt` has one `account<TAB>policy` line per account (`allow-all`, `deny-all` or a
per-table list of read, write and ddl bits); an unlisted account is denied. File formats and the limits
are in the [listener guide](../crates/latticed/README.md#configured-scram-over-tls).

The `lattice account` tool ([#683](https://github.com/moncifem/latticedb/pull/683), issue
[#582](https://github.com/moncifem/latticedb/issues/582)) is on `main`. It creates the verifier file, the
decoy secret, the policy file and bearer-token files (see the
[listener guide](../crates/latticed/README.md#provisioning-with-lattice-account)); it edits files and does not
talk to a running server. The SCRAM-over-TLS path is covered by Rust integration tests
(`crates/latticed/tests`), not by the quickstart smoke test.

### Run in a container

The image `ghcr.io/moncifem/latticedb` (the `nightly` tag until a release is cut) holds `latticed` and
`lattice` on `ubuntu:24.04`. It runs as the user `lattice` (uid 10001), keeps data in the volume
`/var/lib/lattice`, and its entrypoint runs `latticed --stdin-control off` with your arguments, so a
detached container keeps running. With no arguments it prints the flags it needs and exits 2:

```bash
docker run --rm ghcr.io/moncifem/latticedb:nightly
```

A published port needs SCRAM over TLS, because trust mode refuses a non-loopback bind. Keep the account
files in their own volume, outside the data volume (the tool asks for an owner-only directory). Create
them with the image's own `lattice account` (the password is read from stdin or generated and printed
once), and the certificate with a pinned `openssl` command, since the tool does not make TLS keys:

```bash
IMAGE=ghcr.io/moncifem/latticedb:nightly
docker volume create lattice-config && docker volume create lattice-tls
docker run --rm --entrypoint lattice -v lattice-config:/etc/lattice "$IMAGE" account init /etc/lattice
printf '%s\n' "$PASSWORD" | docker run --rm -i --entrypoint lattice -v lattice-config:/etc/lattice "$IMAGE" \
  account add /etc/lattice alice --read notes --write notes --ddl notes --password-stdin

# a throwaway CA and a server certificate for localhost (use your own names and a real CA otherwise)
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca-key.pem -out ca.pem -days 30 -subj "/CN=lattice test CA"
openssl req -newkey rsa:2048 -nodes -keyout server-key.pem -out server.csr -subj "/CN=localhost"
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial -days 30 \
  -extfile server.ext -out server-cert.pem

# the key must be owned by uid 10001 and mode 0600, so copy it in as root with install
for f in server-key.pem:0600 server-cert.pem:0644; do
  name=${f%%:*}; mode=${f##*:}
  docker run --rm --user 0:0 --entrypoint install -v lattice-tls:/etc/lattice-tls -v "$PWD:/src:ro" "$IMAGE" \
    -o 10001 -g 10001 -m "$mode" "/src/$name" "/etc/lattice-tls/$name"
done
```

Start it with the flags the account tool prints, the data and account volumes, and the certificate
volume read-only. The host side is bound to loopback here; publish on `0.0.0.0` only after reading the
listener guide's TLS and account sections:

```bash
docker run -d --name lattice -p 127.0.0.1:5432:5432 \
  -v lattice-data:/var/lib/lattice -v lattice-config:/etc/lattice:ro -v lattice-tls:/etc/lattice-tls:ro \
  "$IMAGE" --data-dir /var/lib/lattice --auth scram-tls --listen 0.0.0.0:5432 \
  --tls-cert /etc/lattice-tls/server-cert.pem --tls-key /etc/lattice-tls/server-key.pem \
  --scram-verifiers /etc/lattice/scram-verifiers --scram-decoy-secret /etc/lattice/scram-decoy-secret \
  --account-policies /etc/lattice/account-policies
PGPASSWORD="$PASSWORD" psql "host=localhost port=5432 user=alice dbname=lattice sslmode=verify-full sslrootcert=ca.pem"
```

`docker stop` gives a clean stop when `latticed` has printed its listening line (wait for it first; a
process that is still recovering ignores SIGTERM as PID 1 and is killed after the stop timeout). Docker
waits 10 seconds by default; use `docker stop -t 40` when connections may take longer to drain. A
`docker start lattice` keeps the same volumes and rows. `docker inspect -f '{{.State.ExitCode}}' lattice`
prints `0` after a clean stop.

Bind mounts keep the host's ownership, not the image's. A host directory mounted at `/var/lib/lattice` or
`/etc/lattice` must be owned by uid 10001 (for example `sudo chown -R 10001:10001 ./data`), or the server
cannot write it; named volumes copy the image's ownership, which is why the examples use them.

What is checked and what is not. `tests/image/smoke.sh` runs the steps above in CI (`image-smoke`) on
the amd64 image: the usage error, the user and volume ownership, the account tool inside the image, SCRAM
over TLS with `sslmode=verify-full`, a wrong password, inserts, a clean `docker stop`, and rows read back
after a restart on the same volumes. It does not run the arm64 image, the published image or a supervisor,
and it does not test a stop during recovery. The image adds no shell or package manager, but the
`ubuntu:24.04` base still ships `/bin/sh` and `apt`; removing them is a separate decision.

## Connect with psql

```bash
psql -h 127.0.0.1 -p 5432 -U dev
```

In SCRAM mode use `"host=... user=ACCOUNT sslmode=verify-full sslrootcert=ca.pem"` and enter the
account's password. The smoke test and the socket tests use `psql` and a plain client that frames
messages itself. Other drivers (psycopg, node-postgres, pgx, JDBC) have not been run
([#413](https://github.com/moncifem/latticedb/issues/413), psql scripts in
[#575](https://github.com/moncifem/latticedb/issues/575), three drivers in
[#576](https://github.com/moncifem/latticedb/issues/576)); results are text format only.

SQL supported: single-table `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `DROP TABLE`, and
transaction control. Joins, aggregates, `GROUP BY`, `DISTINCT` and subqueries answer SQLSTATE `0A000`.
The subset is in [sql-subset.md](sql-subset.md).

### What happens after a restart

Committed rows, tables and their column definitions (types, `NOT NULL`, primary key, `UNIQUE`, defaults) are
recovered from the log ([#573](https://github.com/moncifem/latticedb/issues/573), RFC 0012): `SELECT` works and
the constraints are enforced after a restart. A data directory written by an earlier build, which stored table
names only, refuses its first `CREATE TABLE` (SQLSTATE `55000`) until `latticed` is started with
`--allow-format-upgrade on`; older builds cannot open the directory afterwards and there is no downgrade, so take a
backup first. A new directory needs nothing. Tables that earlier builds created keep their names but cannot be
queried. A quoted column name with a capital letter is refused (`0A000`). The smoke test asserts the restart.

## Connect an agent client

What an agent client can do today: connect as any PostgreSQL client and run the SQL above, as an
account with table-level read, write and ddl grants (SCRAM mode) or allow-all (loopback trust). That
is the whole agent surface on `main`.

What is not shipped, so no agent client can use it yet:

- A usable sync gateway. A WebSocket gateway ([#680](https://github.com/moncifem/latticedb/pull/680)) is on
  `main` behind `--gateway-listen`, with bearer-token authentication
  ([ADR 0031](adr/0031-bearer-token-authentication.md)). It is experimental: loopback only, plaintext (there
  is no TLS on the gateway port, so a non-loopback address is refused), rows only and read only, with no
  idle deadline, no per-principal byte budget and no credential reload. It is not covered by the quickstart
  smoke test.
- MCP over HTTP or stdio. `lattice-sdk` contains the MCP message codec, tool schemas and a grant layer
  ([mcp-v0.md](mcp-v0.md)), and the transport-independent tool backend for `lattice_read` and
  `lattice_write` ([#687](https://github.com/moncifem/latticedb/pull/687)) is on `main`. No MCP transport
  serves it: no listener, token header or process exposes the tools to a client, and `lattice_subscribe` is
  not built.
- `lattice watch`.

No MCP client has been run against `latticed`, so this page gives no client configuration and lists
every client as untested. When a transport lands, each client entry will be added only after someone ran
it, with the date and client version. The design of the live-collaboration slice that these pieces are
building toward is [live-collaboration.md](design/live-collaboration.md); it is a proposal, not shipped
behaviour.

Treat anything stored in the database as data, never as instructions. A row can contain text written by
another principal, and an agent that reads it is exposed to prompt injection. Lattice does not filter
or label stored text, so an agent client has to treat tool and query output as untrusted input and keep
its write access as narrow as the task allows (grant only the tables it needs).

## Experimental and unstable

- The change-feed and sync encodings (RFC 0009 and 0010, ADR 0023 and 0024) are published without an
  accepted contract. They are not frozen and may change in any release.
- The MCP message shapes and tool schemas follow a draft design ([mcp-v0.md](mcp-v0.md)).
- Data directories: the stored row layout and log formats are not frozen. A later alpha may not open
  this alpha's data directory. Do not keep data you need.
- The TypeScript and Python clients in `clients/` are experimental codec clients checked against test
  vectors; they do not talk to a server.
- Durability evidence is simulation, process-kill tests and fuzz and proof jobs listed in the
  [gate pages](gates). It is not real power loss. Linux real-disk numbers are pending
  ([ADR 0011](adr/0011-engine-runtime.md)).

## Not in the alpha

Joins, aggregates, `GROUP BY`, `DISTINCT`, subqueries; binary result formats and JDBC; collaborative
text documents, presence and the workflow arbiter; snapshot-then-tail bootstrap for a feed (a cursor
below the vacuum floor is refused); a policy language, roles or row predicates (Phase 4; the alpha has
table-level grants); checkpoints, branches and online schema change; SCRAM channel binding; resource
subscriptions and MCP notifications; a web UI, schema packs, search, blobs and S3, clusters and the
optional models plugin; Windows.

## Known issues

| Issue | What you will see |
|---|---|
| [#574](https://github.com/moncifem/latticedb/issues/574) | Restart is tested in-process, with a power-cut sweep and psql scripts; the `kill -9` seed sweep with the real binary and the simulator workload are not done yet. Process kill is not power loss. |
| [#413](https://github.com/moncifem/latticedb/issues/413) | Only `psql` and a hand-framed client are tested. No driver matrix; text results only. |
| [#363](https://github.com/moncifem/latticedb/issues/363) | A filtered change feed reveals the count of hidden writes through gaps in cursors and never says a row left a reader's scope. Disclosed, not fixed. |
| [#543](https://github.com/moncifem/latticedb/issues/543) | A feed read still costs more than it should on a large or busy table; a quiet-table reader can get `Unavailable` on every poll after many other-table versions ([#661](https://github.com/moncifem/latticedb/issues/661)). |
| [#674](https://github.com/moncifem/latticedb/issues/674) | `COMMIT` or `ROLLBACK` outside a block, and `BEGIN` inside one, send no PostgreSQL warning (`25P01`, `25001`). |
| [#675](https://github.com/moncifem/latticedb/issues/675) | An unknown type name answers `0A000` instead of `42704`. |
| [#360](https://github.com/moncifem/latticedb/issues/360) | No SCRAM channel binding; a client that skips certificate verification can be relayed. |
| [#361](https://github.com/moncifem/latticedb/issues/361) | No security event log and no failed-authentication backoff for pgwire. |
| No TLS on the gateway | When the gateway ships it listens on loopback only and has no TLS; do not expose it. The pgwire listener can use TLS (SCRAM mode). |
| Token reload | When a gateway uses bearer tokens, they will be read at start only; changing them will need a restart. Nothing reads them today. |
| MCP transport | No HTTP or stdio transport is shipped (above). |
| Containers | The image's entrypoint passes `--stdin-control off` and prints usage with no arguments. A published port needs SCRAM over TLS. `tests/image/smoke.sh` (CI job `image-smoke`) tests the amd64 image only; the `nightly` image predates this entrypoint until the next nightly build. See [Run in a container](#run-in-a-container). |
| Platforms | No Windows (no signal handling or installer). Linux needs glibc as stated in the release notes. |

This table is edited by hand; the open issues and the [gate pages](gates) are the authority.

## Reporting a problem

Bugs and questions go to [issues](https://github.com/moncifem/latticedb/issues) and
[Discussions](https://github.com/moncifem/latticedb/discussions). Vulnerabilities go to the private
channel in [SECURITY.md](../SECURITY.md), never to a public issue.
