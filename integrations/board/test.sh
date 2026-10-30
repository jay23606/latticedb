#!/bin/sh
# End-to-end test of the board helper against a local latticed.
#
# Needs psql and a latticed binary: LATTICE_BIN_DIR/latticed, or latticed on PATH.
# It starts its own server on a free loopback port with a fresh data directory, runs the flow,
# restarts the server on the same data directory, checks what survived, and removes everything
# it created. It never touches another server.
#
#   LATTICE_BIN_DIR=/path/to/bin integrations/board/test.sh
#
# Exit status 0 when every check passes, 1 otherwise.

set -u

HERE=$(cd "$(dirname "$0")" && pwd) || exit 1
BOARD="$HERE/board"

if [ -n "${LATTICE_BIN_DIR:-}" ]; then
  LATTICED="$LATTICE_BIN_DIR/latticed"
else
  LATTICED=$(command -v latticed) || { echo "test: latticed not found; set LATTICE_BIN_DIR" >&2; exit 1; }
fi
[ -x "$LATTICED" ] || { echo "test: $LATTICED is not executable" >&2; exit 1; }
command -v psql >/dev/null 2>&1 || { echo "test: psql not found" >&2; exit 1; }

TAB=$(printf '\t')
NL='
'
PASS=0
FAILS=0
PID=
WORK=$(mktemp -d "${TMPDIR:-/tmp}/board-test.XXXXXX") || exit 1
chmod 700 "$WORK"
DATA="$WORK/data"
LOG=
START_N=0
mkdir -m 700 "$DATA"

cleanup() {
  if [ -n "$PID" ]; then
    kill -TERM "$PID" 2>/dev/null
    wait "$PID" 2>/dev/null
    PID=
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

check() {
  # check NAME COMMAND...: pass when the command succeeds
  ck_name=$1
  shift
  if "$@"; then
    PASS=$((PASS + 1))
    echo "ok   - $ck_name"
  else
    FAILS=$((FAILS + 1))
    echo "FAIL - $ck_name"
  fi
}

expect_eq() {
  # expect_eq NAME EXPECTED ACTUAL
  if [ "$2" = "$3" ]; then
    PASS=$((PASS + 1))
    echo "ok   - $1"
  else
    FAILS=$((FAILS + 1))
    echo "FAIL - $1"
    echo "       expected: $2"
    echo "       actual:   $3"
  fi
}

expect_rc() {
  # expect_rc NAME EXPECTED_STATUS COMMAND...: pass when the command exits with that status
  er_name=$1
  er_want=$2
  shift 2
  "$@" >/dev/null 2>&1
  er_got=$?
  expect_eq "$er_name" "$er_want" "$er_got"
}

pick_port() {
  # A free loopback port. Another process may take it before latticed binds it, which start_server
  # detects and retries.
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
  else
    echo 55432
  fi
}

PORT=$(pick_port) || exit 1
export PGHOST=127.0.0.1
export PGPORT="$PORT"
export PGUSER=dev
export PGPASSWORD=unused-under-trust
export LC_MESSAGES=C

# start_server: start latticed on $PORT with a per-start log, and wait until (a) its own log says it
# is listening on $PORT and (b) SQL answers on $PORT. A process that dies before listening (the
# port was taken) gets a new port and another try.
start_server() {
  ss_try=0
  while [ "$ss_try" -lt 5 ]; do
    ss_try=$((ss_try + 1))
    START_N=$((START_N + 1))
    LOG="$WORK/latticed-$START_N.log"
    "$LATTICED" --data-dir "$DATA" --development-policy allow-all --listen "127.0.0.1:$PORT" \
      --stdin-control off >"$LOG" 2>&1 &
    PID=$!
    n=0
    while [ "$n" -lt 150 ]; do
      if grep -F "latticed listening on 127.0.0.1:$PORT (" "$LOG" >/dev/null 2>&1; then
        if psql -X -q -A -t -c 'SELECT 1' >/dev/null 2>&1; then
          return 0
        fi
      fi
      if ! kill -0 "$PID" 2>/dev/null; then
        wait "$PID" 2>/dev/null
        PID=
        break
      fi
      sleep 0.2
      n=$((n + 1))
    done
    if [ -n "$PID" ]; then
      echo "test: latticed did not report listening on port $PORT; log:" >&2
      cat "$LOG" >&2
      kill -TERM "$PID" 2>/dev/null
      wait "$PID" 2>/dev/null
      PID=
      return 1
    fi
    echo "test: port $PORT was taken before latticed bound it; trying another" >&2
    PORT=$(pick_port) || return 1
    PGPORT="$PORT"
    export PGPORT
  done
  echo "test: latticed did not start after $ss_try tries; logs in $WORK" >&2
  return 1
}

stop_server() {
  kill -TERM "$PID"
  wait "$PID"
  rc=$?
  PID=
  return "$rc"
}

echo "test: latticed=$LATTICED port=$PORT data=$DATA"
start_server || exit 1

# --- init and idempotence -------------------------------------------------------------------
check "init creates the tables" "$BOARD" init
check "init again keeps working" "$BOARD" init
two_inits() {
  ti_p1=
  ti_p2=
  "$BOARD" init >"$WORK/init1.out" 2>&1 &
  ti_p1=$!
  "$BOARD" init >"$WORK/init2.out" 2>&1 &
  ti_p2=$!
  wait "$ti_p1" && wait "$ti_p2"
}
check "two inits at once both succeed" two_inits

# --- authors: required for writes, validated, unique per agent ------------------------------
BOARD_AUTHOR=maint
export BOARD_AUTHOR

T_PLAIN=$("$BOARD" add "Plain task") || FAILS=$((FAILS + 1))
T_BIG=$("$BOARD" add "Urgent refactor" --priority 5 --body "details here") || FAILS=$((FAILS + 1))
T_CHILD=$("$BOARD" add "Child of plain" --parent "$T_PLAIN") || FAILS=$((FAILS + 1))
check "add returns an id" test -n "$T_PLAIN"

LIST=$("$BOARD" list)
FIRST=$(printf '%s\n' "$LIST" | sed -n '2p' | cut -f1)
expect_eq "list puts the highest priority first" "$T_BIG" "$FIRST"
check "list has the header and three tasks" test "$(printf '%s\n' "$LIST" | wc -l | tr -d ' ')" = 4

expect_rc "a write without BOARD_AUTHOR is refused" 1 env -u BOARD_AUTHOR "$BOARD" add "No author"
expect_rc "claim without BOARD_AUTHOR is refused" 1 env -u BOARD_AUTHOR "$BOARD" claim "$T_PLAIN"
expect_rc "note without BOARD_AUTHOR is refused" 1 env -u BOARD_AUTHOR "$BOARD" note "$T_PLAIN" "x"
expect_rc "done without BOARD_AUTHOR is refused" 1 env -u BOARD_AUTHOR "$BOARD" "done" "$T_PLAIN"
expect_rc "decide without BOARD_AUTHOR is refused" 1 env -u BOARD_AUTHOR "$BOARD" decide "$T_PLAIN" "x"
expect_rc "a read needs no BOARD_AUTHOR" 0 env -u BOARD_AUTHOR "$BOARD" list
expect_rc "an author with a space is refused" 1 env BOARD_AUTHOR='agent one' "$BOARD" claim "$T_PLAIN"
check "the refused writes created no task" test "$("$BOARD" list | wc -l | tr -d ' ')" = 4

if "$BOARD" add "Orphan" --parent t0000000000 >/dev/null 2>"$WORK/orphan.err"; then
  FAILS=$((FAILS + 1))
  echo "FAIL - add with a missing parent must fail"
else
  PASS=$((PASS + 1))
  echo "ok   - add with a missing parent fails"
fi
check "a failed add with a missing parent leaves no task" \
  test "$("$BOARD" list | wc -l | tr -d ' ')" = 4

# --- quoting and injection ------------------------------------------------------------------
EVIL="Robert'); DROP TABLE tasks; -- \"q\" ; é 日本語 :'author' :\"x\" %s 'end"
T_EVIL=$("$BOARD" add "$EVIL") || FAILS=$((FAILS + 1))
SHOWN=$("$BOARD" show "$T_EVIL" | grep '^title|' | cut -d'|' -f2-)
expect_eq "a hostile title is stored and read back verbatim" "$EVIL" "$SHOWN"
check "the tasks table survived the hostile title" test "$("$BOARD" list | wc -l | tr -d ' ')" = 5
expect_rc "a title with a backslash is refused" 1 "$BOARD" add 'path C:\temp'
expect_rc "a title with a carriage return is refused" 1 "$BOARD" add "$(printf 'bad\rtitle')"
expect_rc "a title with a bell character is refused" 1 "$BOARD" add "$(printf 'bell\007')"
check "the refused titles stored nothing" test "$("$BOARD" list | wc -l | tr -d ' ')" = 5

# --- claim race: three claimers, five tasks, exactly one winner each ------------------------
race_task() {
  rt_id=$1
  rt_dir="$WORK/race-$rt_id"
  mkdir -p "$rt_dir"
  rt_pids=
  for rt_i in 1 2 3; do
    (BOARD_AUTHOR=racer$rt_i "$BOARD" claim "$rt_id" >"$rt_dir/out.$rt_i" 2>&1
     echo $? >"$rt_dir/rc.$rt_i") &
    rt_pids="$rt_pids $!"
  done
  # Wait only for these claimers: a bare wait would also wait for latticed, which is a child too.
  # shellcheck disable=SC2086
  wait $rt_pids
  rt_winners=0
  rt_winner=
  for rt_i in 1 2 3; do
    if [ "$(cat "$rt_dir/rc.$rt_i")" = 0 ]; then
      rt_winners=$((rt_winners + 1))
      rt_winner=racer$rt_i
    fi
  done
  rt_losers_ok=1
  for rt_i in 1 2 3; do
    if [ "$(cat "$rt_dir/rc.$rt_i")" != 0 ] && [ "$(cat "$rt_dir/rc.$rt_i")" != 2 ]; then
      rt_losers_ok=0
    fi
  done
  [ "$rt_winners" = 1 ] && [ "$rt_losers_ok" = 1 ] || {
    echo "       winners=$rt_winners losers_ok=$rt_losers_ok" >&2
    return 1
  }
  rt_owner=$("$BOARD" show "$rt_id" | grep '^owner|' | cut -d'|' -f2)
  [ "$rt_owner" = "$rt_winner" ] || { echo "       owner=$rt_owner winner=$rt_winner" >&2; return 1; }
  rt_claims=$("$BOARD" since 0 | awk -F"$TAB" -v t="$rt_id" -v k=claim '$2 == t && $4 == k' | wc -l | tr -d ' ')
  [ "$rt_claims" = 1 ] || { echo "       claim events=$rt_claims" >&2; return 1; }
  return 0
}

# twin_claim ID: two claimers that share one name. Both calls succeed (the second is "already
# yours"), and only one claim event is recorded. This is the documented behaviour; it is why each
# agent needs its own name.
twin_claim() {
  tw_id=$1
  tw_dir="$WORK/twin-$tw_id"
  mkdir -p "$tw_dir"
  (BOARD_AUTHOR=twin "$BOARD" claim "$tw_id" >"$tw_dir/out.1" 2>&1; echo $? >"$tw_dir/rc.1") &
  tw_p1=$!
  (BOARD_AUTHOR=twin "$BOARD" claim "$tw_id" >"$tw_dir/out.2" 2>&1; echo $? >"$tw_dir/rc.2") &
  tw_p2=$!
  wait "$tw_p1" "$tw_p2"
  [ "$(cat "$tw_dir/rc.1")" = 0 ] && [ "$(cat "$tw_dir/rc.2")" = 0 ] || return 1
  tw_claims=$("$BOARD" since 0 | awk -F"$TAB" -v t="$tw_id" -v k=claim '$2 == t && $4 == k' | wc -l | tr -d ' ')
  [ "$tw_claims" = 1 ]
}

RACE_TASKS=
for k in 1 2 3 4 5; do
  rid=$("$BOARD" add "Race target $k") || FAILS=$((FAILS + 1))
  RACE_TASKS="$RACE_TASKS $rid"
  check "race $k: exactly one of three distinct claimers wins, one claim event" race_task "$rid"
done
TWIN_T=$("$BOARD" add "Same-name race") || FAILS=$((FAILS + 1))
check "two claimers with the same name: both succeed, one claim event (documented)" twin_claim "$TWIN_T"

FIRST_OUT=$(BOARD_AUTHOR=maint "$BOARD" claim "$T_PLAIN") ; FIRST_RC=$?
expect_eq "the first claim on a plain task wins" "claimed $T_PLAIN" "$FIRST_OUT"
check "a winning claim exits 0" test "$FIRST_RC" = 0
LOSE_OUT=$(BOARD_AUTHOR=racer9 "$BOARD" claim "$T_PLAIN") ; LOSE_RC=$?
check "a claim on a task another author holds exits 2" test "$LOSE_RC" = 2
expect_eq "the refused claim says who holds the task" \
  "not claimed: $T_PLAIN is claimed, owner maint" "$LOSE_OUT"
OWN_OUT=$("$BOARD" claim "$T_PLAIN") ; OWN_RC=$?
expect_eq "claiming your own task again is idempotent" "already yours: $T_PLAIN" "$OWN_OUT"
check "idempotent claim exits 0" test "$OWN_RC" = 0

# --- notes, multi-line rejection, missing task ----------------------------------------------
"$BOARD" note "$T_PLAIN" "first note; it's \"fine\" é" >/dev/null || FAILS=$((FAILS + 1))
check "a note with quotes and unicode is accepted" test -n "$("$BOARD" show "$T_PLAIN" | grep 'first note')"
MULTI="line one${NL}line two"
expect_rc "a multi-line note is refused" 1 "$BOARD" note "$T_PLAIN" "$MULTI"
expect_rc "a note with a control character is refused" 1 "$BOARD" note "$T_PLAIN" "$(printf 'x\001y')"
expect_rc "a note on a missing task fails" 1 "$BOARD" note t0000000000 "x"

# --- concurrent note storm: sixteen writers, five notes each, all must land -----------------
STORM_DIR="$WORK/storm"
mkdir -p "$STORM_DIR"
storm_pids=
w=1
while [ "$w" -le 16 ]; do
  (j=1
   while [ "$j" -le 5 ]; do
     BOARD_AUTHOR=writer$w "$BOARD" note "$T_BIG" "writer $w note $j" >/dev/null 2>>"$STORM_DIR/err" \
       || echo "failed $w $j" >>"$STORM_DIR/failed"
     j=$((j + 1))
   done) &
  storm_pids="$storm_pids $!"
  w=$((w + 1))
done
# shellcheck disable=SC2086
wait $storm_pids
STORM_FAILED=$(cat "$STORM_DIR/failed" 2>/dev/null | wc -l | tr -d ' ')
expect_eq "80 concurrent notes from sixteen writers all land" "0" "$STORM_FAILED"
STORM_EVENTS=$("$BOARD" since 0 | awk -F"$TAB" -v t="$T_BIG" '$2 == t' | wc -l | tr -d ' ')
expect_eq "the task holds all 80 notes plus its add event" "81" "$STORM_EVENTS"
ORDER_BAD=$("$BOARD" since 0 | awk -F"$TAB" -v t="$T_BIG" '
  $2 == t { if (seen && $1 + 0 <= prev) bad++; prev = $1 + 0; seen = 1 }
  END { print bad + 0 }')
expect_eq "event ids for the task rise in order" "0" "$ORDER_BAD"

# --- forced 40001: a write that read the counter before another writer committed ------------
# BOARD_TEST_HOLD_FILE makes the held note stop after its counter bump and wait for a file.
# The test runs another note in that window, so the held note's commit must conflict once.
HOLD="$WORK/hold"
rm -f "$HOLD" "$HOLD.started"
BOARD_TEST_HOLD_FILE="$HOLD" BOARD_AUTHOR=held "$BOARD" note "$T_BIG" "held note" \
  >"$WORK/held.out" 2>"$WORK/held.err" &
HELD_PID=$!
i=0
while [ ! -f "$HOLD.started" ] && [ "$i" -lt 200 ]; do sleep 0.05; i=$((i + 1)); done
BOARD_AUTHOR=other "$BOARD" note "$T_BIG" "note while held" >/dev/null 2>&1 || FAILS=$((FAILS + 1))
: >"$HOLD"
wait "$HELD_PID"
HELD_RC=$?
expect_eq "the held note lands after its conflict" "0" "$HELD_RC"
check "the held note reported a retry on stderr" grep -q "retrying after a write conflict" "$WORK/held.err"
check "the held note is in the log" test -n "$("$BOARD" show "$T_BIG" | grep 'held note')"

# --- done, decide, closed tasks -------------------------------------------------------------
"$BOARD" "done" "$T_CHILD" "closing by hand" >/dev/null || FAILS=$((FAILS + 1))
check "done moves the task to done" test "$("$BOARD" list "done" | cut -f1 | grep -c "$T_CHILD")" = 1
expect_rc "a second done on the same task is refused" 1 "$BOARD" "done" "$T_CHILD" "again"
check "the refused second done wrote no event" \
  test "$("$BOARD" since 0 | awk -F"$TAB" -v t="$T_CHILD" -v k=done '$2 == t && $4 == k' | wc -l | tr -d ' ')" = 1
"$BOARD" claim "$T_CHILD" >/dev/null 2>&1 ; DONE_RC=$?
check "a done task cannot be claimed (exit 2)" test "$DONE_RC" = 2
"$BOARD" decide "$T_BIG" "Use the TEXT id scheme; it's ok" >/dev/null || FAILS=$((FAILS + 1))
check "decide shows up in show" test -n "$("$BOARD" show "$T_BIG" | grep 'Use the TEXT id scheme')"

# --- polling cursor, as a maintainer would use it ------------------------------------------
CURSOR=$("$BOARD" head)
BOARD_AUTHOR=subagent-7 "$BOARD" note "$T_PLAIN" "subagent progress" >/dev/null || FAILS=$((FAILS + 1))
NEW=$("$BOARD" since "$CURSOR")
expect_eq "since returns only the event after the cursor" "1" "$(printf '%s\n' "$NEW" | wc -l | tr -d ' ')"
check "the new event carries the sub-agent author" \
  test "$(printf '%s\n' "$NEW" | cut -f3)" = subagent-7
EMPTY=$("$BOARD" since "$("$BOARD" head)")
expect_eq "since returns nothing when there is nothing new" "" "$EMPTY"

# --- passwords stay out of argv ------------------------------------------------------------
# A spy stands in for psql (BOARD_PSQL). It records its arguments and PGPASSWORD, then runs psql.
SPY="$WORK/psql-spy"
{
  printf '%s\n' '#!/bin/sh'
  printf '%s\n' "printf '%s\\n' \"\$@\" >'$WORK/spy.args'"
  printf '%s\n' "printf 'PGPASSWORD=%s\\n' \"\${PGPASSWORD:-}\" >'$WORK/spy.env'"
  printf '%s\n' 'exec psql "$@"'
} >"$SPY"
chmod 700 "$SPY"
expect_rc "a URL with a password still works" 0 env \
  LATTICE_BOARD_URL="postgresql://dev:s3cret-pw@127.0.0.1:$PORT/lattice" BOARD_PSQL="$SPY" "$BOARD" list
no_password_in_args() {
  ! grep -q s3cret "$WORK/spy.args"
}
check "the password is not in psql's arguments" no_password_in_args
check "the password reaches psql through PGPASSWORD" grep -q '^PGPASSWORD=s3cret-pw$' "$WORK/spy.env"
check "the URL psql gets has no password" grep -q "^postgresql://dev@127.0.0.1:$PORT/lattice\$" "$WORK/spy.args"
expect_rc "a percent-encoded password in the URL is refused" 1 env \
  LATTICE_BOARD_URL="postgresql://dev:p%41ss@127.0.0.1:$PORT/lattice" "$BOARD" list
expect_rc "a password in a key=value connection string is refused" 1 env \
  LATTICE_BOARD_URL="host=127.0.0.1 port=$PORT user=dev password=x" "$BOARD" list
expect_rc "a password query parameter in the URL is refused" 1 env \
  LATTICE_BOARD_URL="postgresql://dev@127.0.0.1:$PORT/lattice?password=x" "$BOARD" list

# --- durability: stop, restart on the same data directory, check --------------------------
BEFORE_LIST=$("$BOARD" list)
BEFORE_HEAD=$("$BOARD" head)
BEFORE_SINCE=$("$BOARD" since 0)
stop_server
expect_eq "latticed stops cleanly on SIGTERM" "0" "$?"
start_server || exit 1
expect_eq "list is the same after a restart" "$BEFORE_LIST" "$("$BOARD" list)"
expect_eq "the event log is the same after a restart" "$BEFORE_SINCE" "$("$BOARD" since 0)"
check "a claim still refuses a task won before the restart" \
  test "$(BOARD_AUTHOR=racer9 "$BOARD" claim "$T_PLAIN" >/dev/null 2>&1; echo $?)" = 2
"$BOARD" note "$T_PLAIN" "written after restart" >/dev/null || FAILS=$((FAILS + 1))
AFTER_HEAD=$("$BOARD" head)
check "the counter continues after a restart" test "$AFTER_HEAD" -gt "$BEFORE_HEAD"

stop_server
expect_eq "latticed stops cleanly after the second start" "0" "$?"

echo
echo "passed: $PASS  failed: $FAILS"
[ "$FAILS" -eq 0 ]
