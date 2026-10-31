#!/usr/bin/env bash
# End-to-end test of dev/lattice-dev.sh against the local Docker daemon: the published image, compose,
# the throwaway CA, SCRAM over TLS and libpq. It runs in its own compose project and its own state dir,
# both picked here, so a stack the owner runs is never touched. It deletes what it created on exit.
#
#   tests/dev/dev-stack.sh
#
# Steps, in the order the script runs them:
#   1.  up with no accounts (no server yet); the state dir and key files have owner-only modes.
#   1b. invalid account names and a state dir that is home or one of its parents are refused.
#   2.  add alice (read, write and ddl on notes) and bob (read on notes); a duplicate name is refused.
#   2b. the published port is bound to 127.0.0.1 only.
#   3.  alice creates the table and inserts two rows; bob reads them and is refused a write.
#   4.  a wrong password is refused.
#   5.  status names the accounts and shows no password.
#   6.  down, then up on the same volumes; the rows are read back.
#   7.  reset without a yes is refused.
#   7b. a failed rename leaves no temp file holding a password.
#   8.  reset --yes removes every container, volume and file the test made.
#
# Needs docker with compose v2, openssl and psql. Passwords are never printed by this script: it reads
# them from the account env files and checks the counts of their appearances in the command output.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
dev=$root/dev/lattice-dev.sh

run_id=$(openssl rand -hex 4)
export LATTICE_DEV_PROJECT=lattice-dev-test-$run_id
export LATTICE_DEV_HOME
LATTICE_DEV_HOME=$(mktemp -d "${TMPDIR:-/tmp}/lattice-dev-test.XXXXXX")
work=$(mktemp -d "${TMPDIR:-/tmp}/lattice-dev-work.XXXXXX")

port=${LATTICE_DEV_TEST_PORT:-25500}
if [ -z "${LATTICE_DEV_TEST_PORT:-}" ]; then
  # The first port from 25500 that nothing listens on.
  while (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; do
    port=$((port + 1))
  done
fi
export LATTICE_DEV_PORT=$port

case "$LATTICE_DEV_PROJECT" in
  lattice-dev-test-*) ;;
  *) echo "refusing to run with project $LATTICE_DEV_PROJECT" >&2; exit 1 ;;
esac

cleanup() {
  status=$?
  "$dev" reset --yes >/dev/null 2>&1 || true
  # Anything reset could not remove: containers and volumes that carry this project's label.
  docker ps -aq --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT" | while IFS= read -r c; do
    docker rm -f "$c" >/dev/null 2>&1 || true
  done
  docker volume ls -q --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT" | while IFS= read -r v; do
    docker volume rm "$v" >/dev/null 2>&1 || true
  done
  rm -rf "$LATTICE_DEV_HOME" "$work"
  exit "$status"
}
trap cleanup EXIT

fail() {
  echo "FAILED  $*" >&2
  exit 1
}

ok() {
  echo "ok      $*"
}

# expect_ok NAME COMMAND...: stdout and stderr go to $work/NAME.{out,err}; on failure the stderr tail
# is shown. Command output is never shown in full, because `account add` prints a password.
expect_ok() {
  local name=$1
  shift
  if ! "$@" >"$work/$name.out" 2>"$work/$name.err"; then
    tail -n 20 "$work/$name.err" >&2
    fail "$name exited non-zero"
  fi
}

# expect_refused NAME COMMAND...: the command must exit non-zero.
expect_refused() {
  local name=$1
  shift
  if "$@" >"$work/$name.out" 2>"$work/$name.err"; then
    fail "$name succeeded but was expected to be refused"
  fi
}

mode_of() {
  if stat -c %a "$1" >/dev/null 2>&1; then stat -c %a "$1"; else stat -f %Lp "$1"; fi
}

password_of() {
  (
    # shellcheck source=/dev/null
    . "$1"
    printf '%s' "$PGPASSWORD"
  )
}

# Containers and volumes that carry this project's label.
project_containers() {
  docker ps -aq --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT"
}
project_volumes() {
  docker volume ls -q --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT"
}

expected_rows=$'1|write the spec|f\n2|check the restart|t'
notes_ddl="CREATE TABLE notes (id INT PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN)"
notes_rows="INSERT INTO notes VALUES (1, 'write the spec', false), (2, 'check the restart', true)"

echo "project $LATTICE_DEV_PROJECT, state dir $LATTICE_DEV_HOME, port $LATTICE_DEV_PORT"

# --- 1. up before any account: the stack is prepared and latticed does not start
expect_ok up "$dev" up
if [ -n "$(docker ps -q --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT" --filter label=com.docker.compose.service=latticed)" ]; then
  fail "latticed started before any account exists"
fi
ok "up_without_accounts_prepares_the_stack_and_does_not_start_the_server"

[ "$(mode_of "$LATTICE_DEV_HOME")" = 700 ] || fail "state dir is not mode 700"
for f in ca-key.pem tls/server-key.pem; do
  [ "$(mode_of "$LATTICE_DEV_HOME/$f")" = 600 ] || fail "$f is not mode 600"
done
ok "state_dir_is_0700_and_key_files_are_0600"

# --- 1b. names and state-dir paths that must be refused before any server or account is touched
long=
printf -v long '%064d' 0
bad_names=(-x --read a/b "bad name" "" "$long" _leading .dotted)
i=0
for n in "${bad_names[@]}"; do
  i=$((i + 1))
  expect_refused "bad-name-$i" "$dev" account add "$n" --read notes
  if ! grep -q 'an account name is' "$work/bad-name-$i.err"; then
    fail "name number $i was not refused by the name check"
  fi
done
expect_refused bad-name-psql "$dev" psql -x -c "SELECT 1"
grep -q 'an account name is' "$work/bad-name-psql.err" || fail "psql accepted the name -x"
ok "invalid_account_names_are_refused_before_the_server_is_used"

home_mode=$(mode_of "$HOME")
symlink_home=$work/home-link
ln -s "$HOME" "$symlink_home"
guard_paths=("$HOME/" "$HOME/." "$HOME/.." "$symlink_home" / "$(dirname "$HOME")")
i=0
for p in "${guard_paths[@]}"; do
  i=$((i + 1))
  expect_refused "home-guard-$i" env LATTICE_DEV_HOME="$p" "$dev" status
done
[ "$(mode_of "$HOME")" = "$home_mode" ] || fail "the home directory mode changed"
# A directory of its own under home is accepted: the guard refuses home and its parents, not home's children.
expect_ok home-child env LATTICE_DEV_HOME="$HOME/.lattice-dev-guard-probe" "$dev" status
[ ! -e "$HOME/.lattice-dev-guard-probe" ] || fail "status created a state dir"
ok "state_dir_that_is_home_or_a_parent_is_refused_in_every_spelling"

# --- 2. two accounts; each add prints its generated password once and restarts the server
expect_ok add-alice "$dev" account add alice --read notes --write notes --ddl notes
alice_pw=$(password_of "$LATTICE_DEV_HOME/accounts/alice.env")
[ "$(grep -cF -- "$alice_pw" "$work/add-alice.out")" = 1 ] || fail "alice's password is not printed exactly once"
grep -qF -- "$alice_pw" "$work/add-alice.err" && fail "alice's password reached stderr"
[ "$(mode_of "$LATTICE_DEV_HOME/accounts/alice.env")" = 600 ] || fail "alice.env is not mode 600"
ok "account_add_prints_the_password_once_and_writes_an_owner_only_env_file"

expect_refused duplicate-alice "$dev" account add alice --read notes
grep -q 'already exists' "$work/duplicate-alice.err" || fail "a duplicate name was not refused as taken"
[ "$(password_of "$LATTICE_DEV_HOME/accounts/alice.env")" = "$alice_pw" ] || fail "a duplicate add changed alice's env file"
ok "duplicate_account_name_is_refused_and_keeps_the_env_file"

expect_ok add-bob "$dev" account add bob --read notes
bob_pw=$(password_of "$LATTICE_DEV_HOME/accounts/bob.env")
[ "$(grep -cF -- "$bob_pw" "$work/add-bob.out")" = 1 ] || fail "bob's password is not printed exactly once"
ok "second_account_added"

# --- 2b. the published port is bound to loopback only (host_ip in compose.yaml). docker port lists every
# host binding: 127.0.0.1 alone passes, and 0.0.0.0 or [::] from a missing host_ip fails.
latticed_id=$(docker ps -q --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT" --filter label=com.docker.compose.service=latticed)
[ -n "$latticed_id" ] || fail "latticed is not running, so its published port cannot be checked"
published=$(docker port "$latticed_id" 5432) || fail "docker port could not list the published port"
[ -n "$published" ] || fail "latticed publishes no port"
while IFS= read -r binding; do
  [ "$binding" = "127.0.0.1:$port" ] || fail "published port is bound to '$binding', wanted 127.0.0.1:$port only"
done <<<"$published"
ok "published_port_is_bound_to_loopback_only"

# --- 3. alice creates the table as an account with ddl and writes rows; bob reads them
expect_ok alice-ddl "$dev" psql alice -X -q -v ON_ERROR_STOP=1 -c "$notes_ddl" -c "$notes_rows"
expect_ok bob-read "$dev" psql bob -X -q -t -A -c "SELECT id, title, done FROM notes ORDER BY id"
[ "$(cat "$work/bob-read.out")" = "$expected_rows" ] || fail "bob read '$(cat "$work/bob-read.out")', wanted the two rows"
ok "alice_creates_the_table_and_bob_reads_its_rows"

expect_refused bob-write "$dev" psql bob -X -q -c "INSERT INTO notes VALUES (3, 'bob write', false)"
grep -q 'permission denied' "$work/bob-write.err" || fail "bob's write was refused for another reason"
ok "bob_without_write_grant_is_refused_a_write"

# --- 4. a wrong password is refused for the same account
sed 's/^PGPASSWORD=.*/PGPASSWORD=not-the-password-of-alice/' "$LATTICE_DEV_HOME/accounts/alice.env" \
  > "$LATTICE_DEV_HOME/accounts/alice-wrong.env"
expect_refused wrong-password "$dev" psql alice-wrong -X -c "SELECT 1"
grep -q 'FATAL' "$work/wrong-password.err" || fail "a refused login did not say FATAL"
grep -qF -- "not-the-password-of-alice" "$work/wrong-password.err" && fail "the wrong password was echoed"
rm -f "$LATTICE_DEV_HOME/accounts/alice-wrong.env"
ok "wrong_password_is_refused"

# --- 5. status names the accounts and shows no password
expect_ok status "$dev" status
if ! grep -q 'accounts: alice bob$' "$work/status.out"; then fail "status does not list both accounts"; fi
grep -qF -- "$alice_pw" "$work/status.out" && fail "status printed alice's password"
ok "status_lists_the_accounts_without_passwords"

# --- 6. down, then up on the same volumes: the rows are durable
expect_ok down "$dev" down
[ -z "$(project_containers)" ] || fail "containers remain after down"
[ -n "$(project_volumes)" ] || fail "down removed the volumes; it must keep them"
expect_ok up-again "$dev" up
[ -n "$(docker ps -q --filter health=healthy --filter "label=com.docker.compose.project=$LATTICE_DEV_PROJECT")" ] \
  || fail "latticed is not healthy after the restart"
expect_ok bob-reread "$dev" psql bob -X -q -t -A -c "SELECT id, title, done FROM notes ORDER BY id"
[ "$(cat "$work/bob-reread.out")" = "$expected_rows" ] || fail "rows after the restart were '$(cat "$work/bob-reread.out")'"
ok "rows_read_back_after_down_and_up_on_the_same_volumes"

# --- 7. reset asks first; a no leaves everything in place
printf 'no\n' >"$work/answer-no"
expect_refused reset-declined "$dev" reset <"$work/answer-no"
[ -f "$LATTICE_DEV_HOME/ca.pem" ] || fail "a declined reset deleted the CA"
ok "reset_without_yes_is_refused_and_keeps_the_data"

# --- 7b. a rename that fails must not leave the password in a temp file. A fake mv on the PATH makes
# the rename fail; the account is created on the server first, so the test's reset removes it too.
mkdir -p "$work/fakebin"
printf '#!/bin/sh\nexit 1\n' > "$work/fakebin/mv"
chmod 0755 "$work/fakebin/mv"
expect_refused rename-fails env PATH="$work/fakebin:$PATH" "$dev" account add carol --read notes
for leftover in "$LATTICE_DEV_HOME"/accounts/.carol.*; do
  [ -e "$leftover" ] && fail "a temp file with the password was left behind by a failed rename"
done
[ ! -e "$LATTICE_DEV_HOME/accounts/carol.env" ] || fail "carol.env exists after a failed rename"
ok "failed_rename_removes_the_temp_file_with_the_password"

# --- 8. reset --yes removes what the test created
expect_ok reset "$dev" reset --yes
[ -z "$(project_containers)" ] || fail "containers remain after reset"
[ -z "$(project_volumes)" ] || fail "volumes remain after reset"
[ ! -e "$LATTICE_DEV_HOME" ] || [ -z "$(ls -A "$LATTICE_DEV_HOME")" ] || fail "state dir still holds files"
ok "reset_removes_containers_volumes_and_state"

echo "dev stack test passed"
