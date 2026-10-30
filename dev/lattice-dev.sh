#!/usr/bin/env bash
# Dev stack for a maintainer agent and its subagents: a real latticed with SCRAM over TLS, published on
# 127.0.0.1 only, with one credential per account. dev/compose.yaml holds the containers and
# docs/alpha.md#dev-stack explains the workflow.
#
#   dev/lattice-dev.sh up
#   dev/lattice-dev.sh account add NAME [--read T1,T2] [--write T1,T2] [--ddl T1,T2]
#   dev/lattice-dev.sh psql NAME [psql arguments]
#   dev/lattice-dev.sh status | logs [docker compose logs options] | down
#   dev/lattice-dev.sh reset [--yes]
#
# Environment, all optional:
#   LATTICE_DEV_HOME     owner-only state dir: CA, server key and certificate, account env files.
#                        Default $HOME/.lattice-dev. Never inside the repository.
#   LATTICE_DEV_PROJECT  compose project name and volume prefix. Default lattice-dev.
#   LATTICE_DEV_PORT     host port on 127.0.0.1. Default 5432.
#   LATTICE_DEV_IMAGE    image to run. Default the published 0.1.0-alpha.1 image, pinned by digest.
#
# Secrets: a generated password is printed once, by `account add`, and written only to that account's
# env file (mode 0600). Passwords never go on a command line: the server reads the account files and
# psql reads PGPASSWORD from the environment of the process this script execs.
set -euo pipefail

die() {
  echo "lattice-dev: $*" >&2
  exit 1
}

usage() {
  sed -n '2,/^set -euo pipefail/{/^set -euo/!p;}' "$0" | sed 's/^# \{0,1\}//'
}

DEV_DIR=$(cd "$(dirname "$0")" && pwd)
COMPOSE_FILE=$DEV_DIR/compose.yaml
HOME_DIR=${LATTICE_DEV_HOME:-$HOME/.lattice-dev}
PROJECT=${LATTICE_DEV_PROJECT:-lattice-dev}
PORT=${LATTICE_DEV_PORT:-5432}
partial_file=""

# Prints the physical path of LATTICE_DEV_HOME: symlinks and . or .. are resolved, so that the checks
# below compare what the paths really are. A directory that does not exist yet is resolved through
# its parent, which must exist. Returns 1 when the path cannot be resolved.
resolve_home_dir() {
  local path=$1 base parent
  if [ -d "$path" ]; then
    (cd -P -- "$path" && pwd -P)
    return
  fi
  base=$(basename -- "$path")
  parent=$(dirname -- "$path")
  case "$base" in
    "" | . | ..) return 1 ;;
  esac
  parent=$(cd -P -- "$parent" 2>/dev/null && pwd -P) || return 1
  printf '%s/%s\n' "$parent" "$base"
}

check_settings() {
  case "$HOME_DIR" in
    /*) ;;
    *) die "LATTICE_DEV_HOME must be an absolute path" ;;
  esac
  if ! HOME_DIR=$(resolve_home_dir "$HOME_DIR"); then
    die "LATTICE_DEV_HOME cannot be resolved: its parent directory must exist"
  fi
  # The state dir is chmod'ed and, on reset, has files deleted in it. It must never be the home
  # directory, a parent of it, or the root, whatever spelling (trailing slash, ., .., symlink) was given.
  local home_real
  home_real=$(cd -P -- "$HOME" && pwd -P) || die "HOME is not a directory"
  case "$home_real/" in
    "$HOME_DIR/"*) die "LATTICE_DEV_HOME must not be $HOME or one of its parent directories" ;;
  esac
  [ "$HOME_DIR" != / ] || die "LATTICE_DEV_HOME must not be the root directory"
  [[ $PROJECT =~ ^[a-z0-9][a-z0-9_-]*$ ]] || die "LATTICE_DEV_PROJECT must be lowercase letters, digits, - or _"
  if ! [[ $PORT =~ ^[0-9]{1,5}$ ]] || [ "$PORT" -lt 1 ] || [ "$PORT" -gt 65535 ]; then
    die "LATTICE_DEV_PORT must be a port number"
  fi
  export LATTICE_DEV_HOME=$HOME_DIR LATTICE_DEV_PROJECT=$PROJECT LATTICE_DEV_PORT=$PORT
  if [ -n "${LATTICE_DEV_IMAGE:-}" ]; then export LATTICE_DEV_IMAGE; fi
}

need_tools() {
  command -v docker >/dev/null 2>&1 || die "docker is not installed"
  docker compose version >/dev/null 2>&1 || die "docker compose (v2) is not available"
  docker info >/dev/null 2>&1 || die "the docker daemon is not running"
  command -v openssl >/dev/null 2>&1 || die "openssl is not installed"
}

# Every compose call names the project, the file and the tools profile. The tools services are only
# started by `docker compose run`.
compose() {
  docker compose --profile tools -p "$PROJECT" -f "$COMPOSE_FILE" "$@"
}

ensure_state_dir() {
  umask 077
  if [ ! -d "$HOME_DIR" ]; then
    mkdir -m 0700 "$HOME_DIR" || die "cannot create $HOME_DIR (its parent must exist)"
  fi
  mkdir -p "$HOME_DIR/tls" "$HOME_DIR/accounts"
  chmod 0700 "$HOME_DIR" "$HOME_DIR/tls" "$HOME_DIR/accounts"
}

# Creates the CA and the server certificate on first use, and renews the server certificate when it
# expires within a day. Sets CERTS_CHANGED=1 when either file was written.
CERTS_CHANGED=0
ensure_certificates() {
  local ca_key=$HOME_DIR/ca-key.pem ca=$HOME_DIR/ca.pem tls=$HOME_DIR/tls
  if [ ! -f "$ca_key" ] || [ ! -f "$ca" ]; then
    rm -f "$ca_key" "$ca" "$HOME_DIR/ca.srl"
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$ca_key" -out "$ca" -days 3650 \
      -subj "/CN=lattice dev CA" -addext "basicConstraints=critical,CA:TRUE" \
      -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null \
      || die "openssl could not create the CA"
    chmod 0600 "$ca_key"
    rm -f "$tls/server-cert.pem"
    CERTS_CHANGED=1
  fi
  if [ ! -f "$tls/server-key.pem" ] || [ ! -f "$tls/server-cert.pem" ] \
    || ! openssl x509 -checkend 86400 -noout -in "$tls/server-cert.pem" >/dev/null 2>&1; then
    local ext=$tls/server.ext csr=$tls/server.csr
    printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > "$ext"
    openssl req -newkey rsa:2048 -nodes -keyout "$tls/server-key.pem" -out "$csr" -subj "/CN=localhost" 2>/dev/null \
      || die "openssl could not create the server key"
    openssl x509 -req -in "$csr" -CA "$ca" -CAkey "$ca_key" -CAserial "$HOME_DIR/ca.srl" -CAcreateserial \
      -days 365 -extfile "$ext" -out "$tls/server-cert.pem" 2>/dev/null \
      || die "openssl could not sign the server certificate"
    chmod 0600 "$tls/server-key.pem"
    rm -f "$csr" "$ext"
    CERTS_CHANGED=1
  fi
}

# Copies the server key and certificate into the tls volume, owned by the server's uid.
install_tls() {
  compose run --rm --no-deps -T tls-install >/dev/null || die "could not copy the TLS files into the volume"
}

# The credential files exist once `lattice account init` has run; the verifier file once the first
# account is added. latticed refuses to start without accounts, so `up` starts it only then.
config_has() {
  compose run --rm --no-deps -T --entrypoint test lattice-cli -f "/etc/lattice/$1" >/dev/null 2>&1
}

is_running() {
  [ -n "$(compose ps --status running -q latticed 2>/dev/null)" ]
}

# Starts latticed, or restarts it when it runs with files that changed, and waits until it is healthy.
start_latticed() {
  if [ "$1" = restart ] && is_running; then
    compose restart latticed >/dev/null || die "could not restart latticed"
  fi
  compose up -d --wait --wait-timeout 120 latticed >/dev/null || die "latticed did not become healthy: see lattice-dev.sh logs"
}

# A name becomes a file name and an argument of lattice and psql, so it must start with a letter or a
# digit (a leading - would read as an option) and hold nothing else.
check_name() {
  [[ ${1:-} =~ ^[A-Za-z0-9][A-Za-z0-9_-]{0,62}$ ]] \
    || die "an account name is 1 to 63 letters, digits, _ or -, and starts with a letter or digit"
}

cmd_up() {
  ensure_state_dir
  ensure_certificates
  install_tls
  if ! config_has scram-decoy-secret; then
    compose run --rm --no-deps -T lattice-cli account init /etc/lattice >/dev/null \
      || die "lattice account init failed"
  fi
  if config_has scram-verifiers; then
    if [ "$CERTS_CHANGED" = 1 ]; then start_latticed restart; else start_latticed keep; fi
    echo "latticed is up on 127.0.0.1:$PORT (project $PROJECT, state $HOME_DIR)"
  else
    echo "stack prepared on 127.0.0.1:$PORT (project $PROJECT, state $HOME_DIR)"
    echo "latticed starts with the first account: lattice-dev.sh account add NAME [--read T] [--write T] [--ddl T]"
  fi
  echo "CA for libpq sslrootcert: $HOME_DIR/ca.pem"
}

cmd_account_add() {
  local name=${1:-}
  check_name "$name"
  shift
  ensure_state_dir
  [ -f "$HOME_DIR/tls/server-cert.pem" ] || die "run lattice-dev.sh up first"
  local envfile=$HOME_DIR/accounts/$name.env
  [ ! -e "$envfile" ] || die "account $name already exists (its env file is $envfile)"

  # The tool prints the generated password once on stdout. Only that line is kept; the rest of its
  # output is not shown. Stderr passes through: the tool never puts a password there.
  local out pw
  out=$(compose run --rm --no-deps -T lattice-cli account add /etc/lattice "$name" "$@") \
    || die "lattice account add failed"
  pw=$(printf '%s\n' "$out" | sed -n 's/^password (shown once, not stored): //p')
  [ -n "$pw" ] || die "lattice account add printed no password"

  umask 077
  # The temp file holds the password until it is renamed. Any exit before the rename removes it.
  partial_file=$(mktemp "$HOME_DIR/accounts/.$name.XXXXXX")
  trap 'if [ -n "$partial_file" ]; then rm -f -- "$partial_file"; fi' EXIT
  printf 'PGHOST=%q\nPGPORT=%q\nPGDATABASE=%q\nPGUSER=%q\nPGPASSWORD=%q\nPGSSLMODE=%q\nPGSSLROOTCERT=%q\n' \
    127.0.0.1 "$PORT" lattice "$name" "$pw" verify-full "$HOME_DIR/ca.pem" > "$partial_file"
  mv -- "$partial_file" "$envfile"
  partial_file=""
  trap - EXIT

  # latticed reads the verifier file at start only, so a running server restarts to see the account.
  start_latticed restart
  echo "account $name added on the server; its libpq settings are in $envfile"
  echo "password (shown once, saved in that file): $pw"
}

cmd_psql() {
  local name=${1:-}
  check_name "$name"
  shift
  local envfile=$HOME_DIR/accounts/$name.env
  [ -f "$envfile" ] || die "no account $name: add it with lattice-dev.sh account add $name"
  command -v psql >/dev/null 2>&1 || die "psql is not installed (brew install libpq)"
  set -a
  # shellcheck source=/dev/null
  . "$envfile"
  set +a
  exec psql "$@"
}

cmd_status() {
  compose ps || true
  local names=() f
  for f in "$HOME_DIR"/accounts/*.env; do
    [ -e "$f" ] || break
    names+=("$(basename "$f" .env)")
  done
  echo "accounts: ${names[*]:-none}"
  echo "state dir: $HOME_DIR"
}

cmd_logs() {
  if [ $# -eq 0 ]; then
    set -- --tail 100
  fi
  compose logs --no-color "$@" latticed
}

cmd_down() {
  compose down --remove-orphans
}

cmd_reset() {
  if [ "${1:-}" != --yes ]; then
    cat >&2 <<EOF
reset deletes every row and every credential of project $PROJECT:
  the containers and the volumes (data, config, tls)
  $HOME_DIR (CA, server key and certificate, account env files)
EOF
    printf 'type yes to continue: ' >&2
    local answer=""
    read -r answer || true
    [ "$answer" = yes ] || die "not reset"
  fi
  compose down --volumes --remove-orphans
  # A volume that compose did not own (created by hand) keeps its project label only if compose made it.
  docker volume ls -q --filter "label=com.docker.compose.project=$PROJECT" | while IFS= read -r v; do
    docker volume rm "$v" >/dev/null
  done
  rm -f "$HOME_DIR/ca-key.pem" "$HOME_DIR/ca.pem" "$HOME_DIR/ca.srl"
  rm -rf "$HOME_DIR/tls" "$HOME_DIR/accounts"
  rmdir "$HOME_DIR" 2>/dev/null || true
  echo "reset project $PROJECT"
}

main() {
  local cmd=${1:-}
  case "$cmd" in
    "" | -h | --help | help)
      usage
      return 0
      ;;
  esac
  shift || true
  check_settings
  need_tools
  case "$cmd" in
    up) cmd_up ;;
    down) cmd_down ;;
    status) cmd_status ;;
    reset) cmd_reset "$@" ;;
    logs) cmd_logs "$@" ;;
    account)
      [ "${1:-}" = add ] || die "usage: lattice-dev.sh account add NAME [--read T] [--write T] [--ddl T]"
      shift
      cmd_account_add "$@"
      ;;
    psql) cmd_psql "$@" ;;
    *) die "unknown subcommand '$cmd' (see lattice-dev.sh --help)" ;;
  esac
}

main "$@"
