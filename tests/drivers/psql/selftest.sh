#!/usr/bin/env bash
# Proves the runner can fail (issue 575): the suite is only evidence if a wrong expectation, an
# empty suite, a dead server and a fixed known failure each turn the run red. CI runs this after
# the real suite; every check below must make run.sh exit non-zero.
#
#   tests/drivers/psql/selftest.sh <path-to-latticed>
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
bin=${1:?usage: selftest.sh <path-to-latticed>}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")  # absolute: the scripts change directory
tmp=$(mktemp -d)
failures=0
trap '[ -n "${LATTICE_PSQL_KEEP:-}" ] || rm -rf "$tmp"' EXIT

expect_red() { # name, text the failure must print, then the command
  local name=$1 reason=$2; shift 2
  if "$@" > "$tmp/$name.log" 2>&1; then
    echo "FAILED  selftest $name: the run passed but must fail" >&2
    failures=$((failures + 1))
  elif ! grep -qF -- "$reason" "$tmp/$name.log"; then
    echo "FAILED  selftest $name: the run failed, but not with '$reason':" >&2
    cat "$tmp/$name.log" >&2
    failures=$((failures + 1))
  else
    echo "ok      selftest $name (run failed as required)"
  fi
}

copy_suite() { # destination
  mkdir -p "$1"
  cp "$here"/[0-9][0-9]_*.sql "$here"/[0-9][0-9]_*.expected "$1/"
  cp -R "$here/known" "$1/known"
}

# 1. an expected file that is wrong by one character
copy_suite "$tmp/wrong"
sed 's/^INSERT 0 2$/INSERT 0 3/' "$here/01_ddl_dml.expected" > "$tmp/wrong/01_ddl_dml.expected"
cmp -s "$here/01_ddl_dml.expected" "$tmp/wrong/01_ddl_dml.expected" \
  && { echo "selftest setup: the edit changed nothing" >&2; exit 1; }
expect_red wrong_expectation "FAILED  01_ddl_dml" env LATTICE_PSQL_SUITE="$tmp/wrong" "$here/run.sh" "$bin" loopback

# 2. no script at all
mkdir -p "$tmp/empty"
expect_red zero_scripts "no psql scripts matched" env LATTICE_PSQL_SUITE="$tmp/empty" "$here/run.sh" "$bin" loopback

# 3. a server that does not start (the binary exits at once)
printf '#!/bin/sh\nexit 1\n' > "$tmp/dead"
chmod +x "$tmp/dead"
expect_red server_does_not_start "latticed exited during startup" "$here/run.sh" "$tmp/dead" loopback

# 4. a known failure whose expectation is what the server returns today
copy_suite "$tmp/fixed"
printf 'ERROR:  0A000\nERROR:  0A000\n' > "$tmp/fixed/known/04_unknown_type.expected"
expect_red known_failure_that_now_passes "now matches PostgreSQL" env LATTICE_PSQL_SUITE="$tmp/fixed" "$here/run.sh" "$bin" loopback

# 5. a known failure whose output changes to something other than its pinned deviation
copy_suite "$tmp/drift"
printf 'ERROR:  42601\nERROR:  0A000\n' > "$tmp/drift/known/04_unknown_type.latticed"
expect_red known_failure_with_new_output "differs from its pinned latticed output" \
  env LATTICE_PSQL_SUITE="$tmp/drift" "$here/run.sh" "$bin" loopback

# 6. a psql that is too old for \bind
printf '#!/bin/sh\necho "psql (PostgreSQL) 15.4"\n' > "$tmp/psql15"
chmod +x "$tmp/psql15"
expect_red psql_too_old "too old" env PSQL="$tmp/psql15" "$here/run.sh" "$bin" loopback

if [ "$failures" -eq 0 ]; then echo "selftest passed"; fi
[ "$failures" -eq 0 ]
