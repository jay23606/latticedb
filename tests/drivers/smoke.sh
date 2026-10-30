#!/usr/bin/env bash
# Driver smoke scripts against a built latticed, text mode (issue 576): node-postgres (pg, node/) and
# pgx (go/). Each script connects over loopback trust, runs the same steps (connect, DDL, insert and
# select with parameters including NULL, bool, int and text, update and delete, a transaction committed
# and one rolled back, an error with its SQLSTATE, a clean close) and prints one JSON object per step.
# smoke_compare.py (standard library only) checks the lines against the hand-written expected.jsonl and
# the pinned known failures in known.jsonl. The Python driver is not here: psycopg 3 is LGPL-3.0 and is
# outside the licence allow list (docs/dependency-policy.md); see the follow-up issue in docs/gates/phase-2.md.
#
#   tests/drivers/smoke.sh <path-to-latticed>
#
# Each driver gets its own latticed on a free loopback port with its own data directory, and that server
# must stop with status 0 after `shutdown`. The run fails when a script is missing, exits non-zero, prints
# a step out of order or with a different value, when a known failure starts to match PostgreSQL, and when
# the comparison accepts a truncated run (a self-test).
#
# Environment: NODE (default node, with node/package-lock.json installed: npm ci in node/), GO (default go;
# go/ fetches its pinned pgx), LATTICE_DRIVERS_WORK (work directory, never deleted), LATTICE_DRIVERS_KEEP
# (keep the temporary work directory). The comparison and the port pick run with python3 (standard library).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
bin=${1:?usage: smoke.sh <path-to-latticed>}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")  # absolute: the scripts change directory
node_bin=${NODE:-node}
go_bin=${GO:-go}

[ -x "$bin" ] || { echo "latticed binary not found or not executable: $bin" >&2; exit 1; }
work_is_temporary=""
if [ -n "${LATTICE_DRIVERS_WORK:-}" ]; then
  work=$LATTICE_DRIVERS_WORK
else
  work=$(mktemp -d)
  [ -n "${LATTICE_DRIVERS_KEEP:-}" ] || work_is_temporary=yes
fi
mkdir -p "$work"
echo "work directory: $work"
"$node_bin" --version
"$go_bin" version

server_pid=""

# start_server <dir>: latticed on a free loopback port in <dir>/data, with a FIFO for stdin so that
# stop_server can send `shutdown`. Sets $port. Waits until latticed listens.
start_server() {
  local dir=$1
  port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  mkdir -p "$dir/data"
  chmod 700 "$dir/data"
  rm -f "$dir/ctl"
  mkfifo "$dir/ctl"
  local log="$dir/server.log"
  "$bin" --data-dir "$dir/data" --listen "127.0.0.1:$port" --development-policy allow-all \
    < "$dir/ctl" > "$log" 2>&1 &
  server_pid=$!
  # Open the FIFO read-write: a write-only open waits for a reader, and latticed may die before it opens
  # its stdin, which would block this script forever. Read-write never blocks; the death is caught below.
  exec 3<> "$dir/ctl"
  local _
  for _ in $(seq 1 120); do
    grep -q 'latticed listening on' "$log" && return 0
    kill -0 "$server_pid" 2>/dev/null || { echo "latticed exited during startup:" >&2; cat "$log" >&2; return 1; }
    sleep 0.5
  done
  echo "latticed did not start listening:" >&2
  cat "$log" >&2
  return 1
}

# stop_server: `shutdown` on stdin, then the process must exit with status 0.
stop_server() {
  [ -n "$server_pid" ] || return 0
  echo shutdown >&3
  exec 3>&-
  local _ status
  for _ in $(seq 1 80); do
    if ! kill -0 "$server_pid" 2>/dev/null; then
      status=0
      wait "$server_pid" || status=$?
      server_pid=""
      [ "$status" -eq 0 ] || { echo "latticed exited with status $status after shutdown" >&2; return 1; }
      return 0
    fi
    sleep 0.5
  done
  echo "latticed did not stop after shutdown" >&2
  kill "$server_pid" 2>/dev/null || true
  server_pid=""
  return 1
}

cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
  fi
  if [ -n "$work_is_temporary" ]; then
    rm -rf "$work"
  fi
}
trap cleanup EXIT

failed=0

go_smoke() {
  (cd "$here/go" && "$go_bin" run .)
}

# run_driver <name> <command...>: runs one driver script against a fresh latticed and compares it.
run_driver() {
  local name=$1; shift
  local dir="$work/$name" actual="$work/$name/actual.jsonl" status=0
  mkdir -p "$dir"
  echo "== $name"
  if ! start_server "$dir"; then
    echo "FAILED  $name: latticed did not start" >&2
    failed=$((failed + 1))
    return
  fi
  export LATTICE_DSN="postgresql://lattice@127.0.0.1:$port/lattice?sslmode=disable"
  "$@" > "$actual" 2> "$dir/driver.stderr" || status=$?
  unset LATTICE_DSN
  if ! stop_server; then
    echo "FAILED  $name: latticed did not stop cleanly" >&2
    failed=$((failed + 1))
  fi
  if [ "$status" -ne 0 ]; then
    echo "FAILED  $name: the script exited with status $status:" >&2
    cat "$dir/driver.stderr" >&2
    failed=$((failed + 1))
    return
  fi
  if ! python3 -I "$here/smoke_compare.py" "$here/$name/expected.jsonl" "$here/$name/known.jsonl" "$actual"; then
    failed=$((failed + 1))
    return
  fi
  # The comparison must be able to fail: a run with its last step missing must not pass.
  sed '$d' "$actual" > "$dir/truncated.jsonl"
  if python3 -I "$here/smoke_compare.py" "$here/$name/expected.jsonl" "$here/$name/known.jsonl" "$dir/truncated.jsonl" > /dev/null 2>&1; then
    echo "FAILED  $name: the comparison accepted a truncated run (self-test)" >&2
    failed=$((failed + 1))
  fi
}

[ -f "$here/node/smoke.mjs" ] || { echo "missing node/smoke.mjs" >&2; exit 1; }
[ -f "$here/go/main.go" ] || { echo "missing go/main.go" >&2; exit 1; }

run_driver node "$node_bin" "$here/node/smoke.mjs"
run_driver go go_smoke

if [ "$failed" -ne 0 ]; then
  echo "driver smoke: $failed failed" >&2
  exit 1
fi
echo "driver smoke: all drivers passed their expected output"
