#!/usr/bin/env bash
# Smoke test of the container image (issue #587). Runs the image the way docs/alpha.md documents:
#   - `docker run IMAGE` with no arguments prints the required flags and exits 2 (not a silent exit)
#   - the process runs as uid 10001, and fresh named volumes are owned by that user
#   - `lattice account` provisions SCRAM credentials inside the image, into a volume
#   - latticed serves SCRAM-SHA-256 over TLS on 0.0.0.0:5432 in the container; psql with
#     sslmode=verify-full inserts and selects; a wrong password is refused
#   - `docker stop` gives a clean stop (exit 0 within 10 seconds) and a restart on the same
#     volumes reads the committed rows back, and the primary key still refuses a duplicate
#
#   tests/image/smoke.sh IMAGE [work-dir]
#
# Needs docker, openssl and psql. The certificate and key are generated here; the key is copied into
# a volume with `install -o 10001` (run as root inside the image, with the base image's install
# binary), so the non-root server can read it with mode 0600. The password comes from stdin and is
# never printed. Logs of the container go to work-dir/logs; the generated keys in work-dir/gen and, when
# the script made its own work dir, the whole work dir are removed on exit.
set -euo pipefail

image=${1:?usage: smoke.sh IMAGE [work-dir]}
port=${SMOKE_PORT:-5432}
uid=10001
run_id=$(openssl rand -hex 4)
name=latticed-smoke-$run_id
vol_data=lattice-smoke-data-$run_id
vol_config=lattice-smoke-config-$run_id
vol_tls=lattice-smoke-tls-$run_id
vol_own_data=lattice-smoke-own-data-$run_id
vol_own_cfg=lattice-smoke-own-cfg-$run_id

own_work=0
if [ -n "${2:-}" ]; then work=$2; else work=$(mktemp -d); own_work=1; fi
dir=$work/image-smoke
rm -rf "$dir"
mkdir -p "$dir/gen" "$dir/logs"
chmod 700 "$dir" "$dir/gen"
gen=$dir/gen
logs=$dir/logs

cleanup() {
  status=$?
  if docker inspect "$name" >/dev/null 2>&1; then
    docker logs "$name" > "$logs/latticed.log" 2>&1 || true
    docker rm -f "$name" >/dev/null 2>&1 || true
  fi
  # Runs on failure too, so every volume and key this script made is removed before it exits.
  docker volume rm "$vol_data" "$vol_config" "$vol_tls" "$vol_own_data" "$vol_own_cfg" >/dev/null 2>&1 || true
  rm -rf "$gen"
  if [ "$own_work" = 1 ]; then rm -rf "$work"; fi
  exit "$status"
}
trap cleanup EXIT

fail() { echo "FAILED  $*" >&2; exit 1; }
ok() { echo "ok      $*"; }

# --- 1. no arguments: a usage error that names the flags, exit 2
status=0
out=$(docker run --rm "$image" 2>&1) || status=$?
[ "$status" -eq 2 ] || fail "no-argument run exited $status, wanted 2: $out"
case "$out" in
  *--data-dir*--auth*--tls-cert*--scram-verifiers*) ok "no_arguments_prints_usage_and_exits_2" ;;
  *) fail "usage text does not name the required flags: $out" ;;
esac

# --- 2. arguments reach latticed behind the entrypoint
case "$(docker run --rm "$image" --version)" in
  "latticed "[0-9]*) ok "version_flag_passes_through_the_entrypoint" ;;
  *) fail "--version through the entrypoint printed something else" ;;
esac

# --- 3. non-root user, and fresh named volumes owned by it
[ "$(docker run --rm --entrypoint id "$image" -u)" = "$uid" ] || fail "the image does not run as uid $uid"
ok "runs_as_uid_10001"
for pair in "$vol_own_data:/var/lib/lattice" "$vol_own_cfg:/etc/lattice"; do
  vol=${pair%%:*}
  path=${pair#*:}
  got=$(docker run --rm -v "$vol:$path" --entrypoint stat "$image" -c '%u:%g %a' "$path")
  [ "$got" = "$uid:$uid 700" ] || fail "fresh volume at $path is '$got', wanted '$uid:$uid 700'"
done
ok "fresh_named_volumes_are_owned_by_uid_10001_mode_0700"

# --- 4. credentials from the image: account init and add, the password on stdin
docker run --rm --entrypoint lattice -v "$vol_config:/etc/lattice" "$image" \
  account init /etc/lattice > "$logs/account-init.out" || fail "lattice account init failed"
pw=$(openssl rand -hex 24)
printf '%s\n' "$pw" | docker run --rm -i --entrypoint lattice -v "$vol_config:/etc/lattice" "$image" \
  account add /etc/lattice smoke --read notes --write notes --ddl notes --password-stdin \
  > "$logs/account-add.out" || fail "lattice account add failed"
ok "lattice_account_init_and_add_inside_the_image"

# --- 5. certificate: a throwaway CA and a server certificate for localhost and 127.0.0.1
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$gen/ca-key.pem" 2>/dev/null
openssl req -x509 -new -key "$gen/ca-key.pem" -subj "/CN=lattice image smoke CA" -days 2 -out "$gen/ca.pem"
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$gen/server-key.pem" 2>/dev/null
openssl req -new -key "$gen/server-key.pem" -subj "/CN=localhost" -out "$gen/server.csr"
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > "$gen/server.ext"
openssl x509 -req -in "$gen/server.csr" -CA "$gen/ca.pem" -CAkey "$gen/ca-key.pem" -CAcreateserial \
  -days 2 -extfile "$gen/server.ext" -out "$gen/server-cert.pem" 2>/dev/null
docker run --rm --user 0:0 --entrypoint install -v "$vol_tls:/etc/lattice-tls" -v "$gen:/gen:ro" "$image" \
  -o "$uid" -g "$uid" -m 0600 /gen/server-key.pem /etc/lattice-tls/server-key.pem
docker run --rm --user 0:0 --entrypoint install -v "$vol_tls:/etc/lattice-tls" -v "$gen:/gen:ro" "$image" \
  -m 0644 /gen/server-cert.pem /etc/lattice-tls/server-cert.pem
ok "tls_key_installed_for_uid_10001_mode_0600"

# --- 6. serve: SCRAM over TLS, durable data on the data volume
serve() {
  docker run -d --name "$name" -p "127.0.0.1:$port:5432" \
    -v "$vol_data:/var/lib/lattice" -v "$vol_config:/etc/lattice:ro" -v "$vol_tls:/etc/lattice-tls:ro" \
    "$image" --data-dir /var/lib/lattice --auth scram-tls --listen 0.0.0.0:5432 \
    --tls-cert /etc/lattice-tls/server-cert.pem --tls-key /etc/lattice-tls/server-key.pem \
    --scram-verifiers /etc/lattice/scram-verifiers --scram-decoy-secret /etc/lattice/scram-decoy-secret \
    --account-policies /etc/lattice/account-policies >/dev/null
  local logtext
  for _ in $(seq 1 120); do
    logtext=$(docker logs "$name" 2>&1 || true)
    case "$logtext" in
      *"latticed listening on 0.0.0.0:5432"*) return 0 ;;
    esac
    [ "$(docker inspect -f '{{.State.Running}}' "$name")" = true ] || break
    sleep 0.5
  done
  fail "latticed did not start listening in the container"
}

psql_as() { # password, then psql arguments
  local pass=$1
  shift
  PGPASSWORD=$pass PGPASSFILE=/nonexistent psql \
    "host=localhost hostaddr=127.0.0.1 port=$port user=smoke dbname=lattice sslmode=verify-full sslrootcert=$gen/ca.pem connect_timeout=10" \
    -X -q -t -A -v ON_ERROR_STOP=1 -v VERBOSITY=sqlstate "$@"
}

stop_clean() {
  local start elapsed code
  start=$(date +%s)
  docker stop -t 10 "$name" >/dev/null
  elapsed=$(( $(date +%s) - start ))
  code=$(docker inspect -f '{{.State.ExitCode}}' "$name")
  [ "$code" = 0 ] || fail "docker stop left exit code $code (137 means it was killed)"
  [ "$elapsed" -le 10 ] || fail "docker stop took ${elapsed}s"
  docker logs "$name" 2>&1 | grep -qF 'stop signal received' || fail "no clean-stop line in the log"
}

serve
ok "latticed_listens_on_0_0_0_0_5432_with_scram_tls"

if psql_as "not the password" -c 'SELECT 1' >/dev/null 2>&1; then
  fail "a wrong password was accepted"
fi
ok "wrong_password_is_refused"

psql_as "$pw" -c "CREATE TABLE notes (id INT PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN);" \
  -c "INSERT INTO notes VALUES (1, 'write the spec', false);" \
  -c "INSERT INTO notes VALUES (2, 'check the restart', true);" >/dev/null
got=$(psql_as "$pw" -c 'SELECT id, title, done FROM notes ORDER BY id;')
want=$'1|write the spec|f\n2|check the restart|t'
[ "$got" = "$want" ] || fail "rows before the restart were '$got'"
ok "psql_verify_full_inserts_and_selects"

stop_clean
ok "docker_stop_is_a_clean_stop_within_10_seconds"

docker rm "$name" >/dev/null
serve
got=$(psql_as "$pw" -c 'SELECT id, title, done FROM notes ORDER BY id;')
[ "$got" = "$want" ] || fail "rows after the restart on the same volume were '$got'"
dup=$(psql_as "$pw" -c "INSERT INTO notes VALUES (1, 'again', true);" 2>&1 || true)
case "$dup" in
  *23505*) ;;
  *) fail "a duplicate key after the restart was not refused with 23505: $dup" ;;
esac
ok "rows_read_back_after_restart_on_the_same_volume"

stop_clean
ok "second_docker_stop_is_clean"
echo "image smoke passed for $image"
