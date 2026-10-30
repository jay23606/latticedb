#!/usr/bin/env bash
# Runs the psql script suite against a built latticed (issue 575).
#
#   tests/drivers/psql/run.sh <path-to-latticed> [loopback|scram-tls|all]
#
# loopback: starts latticed with --data-dir under loopback trust, runs every NN_*.sql through psql
#   over stdin and diffs the combined stdout and stderr against the checked-in NN_*.expected. The
#   expected files are written by hand from PostgreSQL's documented psql output and command tags,
#   never recorded from latticed.
#   Cases in known/ are statements PostgreSQL accepts and latticed does not yet. Each names its issue
#   in NN_*.issue and has two files besides the script: NN_*.expected (what PostgreSQL prints) and
#   NN_*.latticed (the deviation latticed has today, checked by hand against the issue). A known case
#   is reported as xfail only while latticed's output equals the .latticed file exactly. If it starts
#   to match .expected the run fails (move the case out of known/); any other output, a new
#   regression hiding behind the old one, fails the run too.
# scram-tls: see scram_tls.sh.
#
# Environment: PSQL (client, default psql), LATTICE_PSQL_PORT (default: a free port),
# LATTICE_PSQL_WORK (work directory, never deleted; default: a temporary one, deleted on exit
# unless LATTICE_PSQL_KEEP is set).
#
# The run fails when no script matched, when the server did not start, and when it did not stop
# cleanly. It needs only psql 16 or later (for \bind), bash, python3 and the latticed binary.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# The scripts to run. Only selftest.sh points this somewhere else.
suite=${LATTICE_PSQL_SUITE:-$here}
bin=${1:?usage: run.sh <path-to-latticed> [loopback|scram-tls|all]}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")  # absolute: the scripts change directory
mode=${2:-all}
psql_bin=${PSQL:-psql}

# shellcheck source=lib.sh
. "$here/lib.sh"

[ -x "$bin" ] || { echo "latticed binary not found or not executable: $bin" >&2; exit 1; }
require_psql_16
pick_port
if [ -n "${LATTICE_PSQL_WORK:-}" ]; then
  work=$LATTICE_PSQL_WORK
else
  work=$(mktemp -d)
  own_dirs="$own_dirs $work"
fi
mkdir -p "$work"
echo "work directory: $work"

# A known failure only counts when psql reached the server: a refused connection also "differs".
connected() { ! grep -qE 'connection to server|could not connect|^psql: error' "$1"; }

failed=0 passed=0 xfailed=0

# check_known <label> <expected> <pinned> <actual> <issue file> <how to say "now fixed">
check_known() {
  local label=$1 expected=$2 pinned=$3 actual=$4 issuefile=$5 fixed=$6 issue
  issue=$(tr -d '[:space:]' < "$issuefile")
  case "$issue" in '#'[0-9]*) ;; *) echo "FAILED  known/$label has no issue number" >&2; failed=$((failed + 1)); return ;; esac
  if diff -q "$expected" "$actual" >/dev/null; then
    echo "FAILED  known/$label now matches PostgreSQL: $fixed ($issue)" >&2
    failed=$((failed + 1))
  elif ! connected "$actual"; then
    echo "FAILED  known/$label: psql could not connect" >&2; failed=$((failed + 1))
  elif diff -u "$pinned" "$actual"; then
    echo "xfail   known/$label ($issue)"; xfailed=$((xfailed + 1))
  else
    echo "FAILED  known/$label differs from its pinned latticed output above, so something other than $issue changed" >&2
    failed=$((failed + 1))
  fi
}

run_loopback() {
  shopt -s nullglob
  local scripts=("$suite"/[0-9][0-9]_*.sql)
  local known=("$suite"/known/[0-9][0-9]_*.sql)
  [ "${#scripts[@]}" -gt 0 ] || { echo "no psql scripts matched in $suite" >&2; return 1; }

  start_server "$work/loopback" first --listen "127.0.0.1:$port" --development-policy allow-all || return 1
  local sql name
  for sql in "${scripts[@]}"; do
    name=$(basename "$sql" .sql)
    run_psql_script "$sql" "$work/loopback/$name.actual"
    if diff -u "${sql%.sql}.expected" "$work/loopback/$name.actual"; then
      echo "ok      $name"; passed=$((passed + 1))
    else
      echo "FAILED  $name" >&2; failed=$((failed + 1))
    fi
  done
  for sql in ${known[@]+"${known[@]}"}; do
    name=$(basename "$sql" .sql)
    run_psql_script "$sql" "$work/loopback/known-$name.actual"
    check_known "$name" "${sql%.sql}.expected" "${sql%.sql}.latticed" "$work/loopback/known-$name.actual" \
      "${sql%.sql}.issue" "move it out of known/ and drop its issue note"
  done

  # Restart on the same data directory. Run 1 must match exactly; run 2 is a known failure until
  # column definitions are durable.
  run_psql_script "$suite/known/restart_a.sql" "$work/loopback/restart_a.actual"
  if diff -u "$suite/known/restart_a.expected" "$work/loopback/restart_a.actual"; then
    echo "ok      known/restart_a"; passed=$((passed + 1))
  else
    echo "FAILED  known/restart_a" >&2; failed=$((failed + 1))
  fi
  stop_server || failed=$((failed + 1))
  start_server "$work/loopback" second --listen "127.0.0.1:$port" --development-policy allow-all || return 1
  run_psql_script "$suite/known/restart_b.sql" "$work/loopback/restart_b.actual"
  check_known restart_b "$suite/known/restart_b.expected" "$suite/known/restart_b.latticed" \
    "$work/loopback/restart_b.actual" "$suite/known/restart.issue" "move it into the numbered scripts"
  stop_server || failed=$((failed + 1))
  echo "loopback: $passed passed, $failed failed, $xfailed known failing"
  [ "$passed" -gt 0 ] && [ "$failed" -eq 0 ]
}

export LATTICE_PSQL_PORT=$port
case "$mode" in
  loopback) run_loopback ;;
  scram-tls) bash "$here/scram_tls.sh" "$bin" "$work" ;;
  all) run_loopback; bash "$here/scram_tls.sh" "$bin" "$work" ;;
  *) echo "unknown mode: $mode" >&2; exit 1 ;;
esac
