#!/usr/bin/env bash
# shellcheck disable=SC2154
# Shared helpers for run.sh and scram_tls.sh. Sourced, not executed. Expects $bin (latticed),
# $psql_bin and $port to be set by the caller.

# psql and libpq translate "ERROR", "(1 row)" and their own messages; the expected files are English.
export LC_ALL=C

server_pid=""
server_dir=""
# Temporary directories this process made (space separated, no spaces in names). They are removed on
# exit unless LATTICE_PSQL_KEEP is set; a directory given through LATTICE_PSQL_WORK is never in it.
own_dirs=""

# require_psql_16: \bind needs libpq 16. Prints the version either way.
require_psql_16() {
  command -v "$psql_bin" >/dev/null || { echo "psql not found (set PSQL)" >&2; exit 1; }
  local version major
  version=$("$psql_bin" --version)
  echo "psql: $version"
  major=$(printf '%s' "$version" | grep -Eo '[0-9]+' | head -1)
  case "$major" in
    '' | *[!0-9]*) echo "cannot read the psql major version from: $version" >&2; exit 1 ;;
  esac
  [ "$major" -ge 16 ] || { printf "psql %s is too old: the scripts use the bind command, which needs psql 16 or later\\n" "$major" >&2; exit 1; }
}

# pick_port: sets $port to LATTICE_PSQL_PORT, or to a port the OS just handed out. The port is
# released before latticed binds it, so another process could take it in between; that shows up as
# a failed start, never as a pass.
pick_port() {
  if [ -n "${LATTICE_PSQL_PORT:-}" ]; then
    port=$LATTICE_PSQL_PORT
  else
    port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  fi
}

# start_server <dir> <label> <latticed args...>
# Starts latticed on <dir>/data. A FIFO holds its stdin open, as a terminal does, until
# stop_server writes `shutdown`. Waits for the listening line and fails when the process exits
# first or never listens, so a server that did not start can not look like a passing run.
start_server() {
  server_dir=$1; local label=$2; shift 2
  mkdir -p "$server_dir"
  rm -f "$server_dir/ctl"
  mkfifo "$server_dir/ctl"
  local log="$server_dir/server-$label.log"
  "$bin" --data-dir "$server_dir/data" "$@" < "$server_dir/ctl" > "$log" 2>&1 &
  server_pid=$!
  exec 3> "$server_dir/ctl"
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

kill_server() {
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
}
cleanup() {
  kill_server
  if [ -z "${LATTICE_PSQL_KEEP:-}" ]; then
    local d
    for d in $own_dirs; do rm -rf "$d"; done
  fi
}
trap cleanup EXIT

# psql_loopback: reads statements on stdin; trust authentication on loopback, no psqlrc, unaligned output,
# NULL shown as [null], errors as bare SQLSTATE codes.
psql_loopback() {
  LC_ALL=C PGPASSFILE="${server_dir:-/nonexistent}/no-pgpass" PGCONNECT_TIMEOUT=10 \
    "$psql_bin" -X -A -P null='[null]' -v VERBOSITY=sqlstate \
    -h 127.0.0.1 -p "$port" -U dev -d dev
}

# run_psql_script <script.sql> <output file>: statements on stdin so psql prints no file names, and
# stdout and stderr in one file in the order psql wrote them.
run_psql_script() {
  psql_loopback < "$1" > "$2" 2>&1 || true
}
