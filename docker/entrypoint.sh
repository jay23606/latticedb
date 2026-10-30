#!/bin/sh
# Entrypoint of the container image. Two jobs, both needed for `docker run IMAGE` to be usable:
#   - with no arguments, print what to pass and exit 2 instead of starting a server that stops at
#     once (the listener is loopback-only in trust mode, so a published port needs SCRAM over TLS);
#   - otherwise run latticed with --stdin-control off, because a detached container gets /dev/null
#     on stdin and end-of-file would stop it. A later --stdin-control on on the command line wins.
# The shell only does this check and then `exec`s latticed, which becomes PID 1 and receives SIGTERM.
set -eu

if [ "$#" -eq 0 ]; then
  cat >&2 <<'USAGE'
latticed: no arguments. Pass the flags for the mode you run. A published port needs SCRAM over TLS.

Required for a durable server (the files come from a volume or a bind mount, see the image docs):
  --data-dir /var/lib/lattice
  --auth scram-tls --listen 0.0.0.0:5432
  --tls-cert FILE --tls-key FILE                         PEM chain and key, key mode 0600
  --scram-verifiers FILE --scram-decoy-secret FILE       from `lattice account init`
  --account-policies FILE                                from `lattice account add`

Provision the account files with: docker run --rm -v CONFIG:/etc/lattice --entrypoint lattice IMAGE account init /etc/lattice
Full option list: docker run --rm IMAGE --help
USAGE
  exit 2
fi

exec /usr/local/bin/latticed --stdin-control off "$@"
