#!/bin/sh
# Installs the Lattice command line tools (`lattice` and `latticed`) from a GitHub release.
#
#   sh install.sh [--prefix DIR] [VERSION]
#
# VERSION is a release version such as 0.1.0-alpha.1 (a leading v is accepted) or `nightly`.
# Without it the newest pre-release tag is looked up through the GitHub API; the script fails
# with a message when there is none, and it never falls back to a mutable name.
# The binaries go to DIR/bin (default DIR is ~/.local). The script needs no sudo, edits no shell
# profile and never updates itself. Supported: macOS arm64, Linux x86_64, Linux arm64.
#
# Before anything is installed the archive must match its line in the release SHA256SUMS, and
# when the `gh` CLI is present its build provenance attestation must verify too.
#
# Test hooks (used by scripts/test_install.sh, not needed otherwise):
#   LATTICE_INSTALL_BASE  directory URL that replaces .../releases/download/<tag>
#   LATTICE_INSTALL_API   URL that replaces the GitHub releases listing

set -eu

REPO="moncifem/latticedb"
API="${LATTICE_INSTALL_API:-https://api.github.com/repos/$REPO/releases?per_page=30}"

say() { printf '%s\n' "install.sh: $*"; }
die() { printf '%s\n' "install.sh: error: $*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
usage: install.sh [--prefix DIR] [VERSION]

  --prefix DIR   install into DIR/bin (default: $HOME/.local, so $HOME/.local/bin)
  VERSION        for example 0.1.0-alpha.1 or nightly (default: the newest pre-release tag)
USAGE
}

prefix=""
version=""
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --prefix)
      [ $# -ge 2 ] || die "--prefix needs a directory"
      prefix="$2"; shift 2 ;;
    --prefix=*) prefix="${1#--prefix=}"; shift ;;
    -*) usage >&2; die "unknown option: $1" ;;
    *)
      [ -z "$version" ] || die "only one version may be given"
      version="$1"; shift ;;
  esac
done

if [ -z "$prefix" ]; then
  [ -n "${HOME:-}" ] || die "HOME is not set; pass --prefix DIR"
  prefix="$HOME/.local"
fi

# Platform. The release workflow builds exactly these three targets.
os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Darwin/arm64) target="aarch64-apple-darwin" ;;
  Linux/x86_64|Linux/amd64) target="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64|Linux/arm64) target="aarch64-unknown-linux-gnu" ;;
  *)
    die "unsupported platform $os/$arch. Releases are built for macOS arm64 (Darwin/arm64), Linux x86_64 and Linux arm64 only. Nothing was installed."
    ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"
# GNU tar runs gzip as a child process to read a .tar.gz.
command -v gzip >/dev/null 2>&1 || die "gzip is required"
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  die "sha256sum or shasum is required to check the download"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lattice-install.XXXXXX") || die "cannot create a temporary directory"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# Only https, except when a test override points at a local file:// directory.
if [ -n "${LATTICE_INSTALL_BASE:-}" ] || [ -n "${LATTICE_INSTALL_API:-}" ]; then
  allowed_proto="=https,file"
else
  allowed_proto="=https"
fi
fetch() { curl -fsSL --proto "$allowed_proto" --proto-redir "=https" "$@"; }

if [ -z "$version" ]; then
  say "looking up the newest pre-release of $REPO"
  fetch "$API" -o "$tmp/releases.json" \
    || die "could not read the release list from $API (network down or rate limited). Pass a version, for example: sh install.sh 0.1.0-alpha.1"
  # The listing is newest first. Tags such as `nightly` are not versions and are skipped.
  version=$(grep '"tag_name"' "$tmp/releases.json" \
    | sed 's/.*"tag_name"[^"]*"\([^"]*\)".*/\1/' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+-[0-9A-Za-z.-]+$' \
    | head -n 1) || true
  [ -n "$version" ] || die "no pre-release tag (vX.Y.Z-alpha.N or similar) has been published yet. Nothing was installed; pass a version once one exists."
  say "newest pre-release: $version"
fi

version="${version#v}"
if [ "$version" = "nightly" ]; then
  tag="nightly"
else
  # The character check also refuses a newline, which a line-based grep would let through.
  case "$version" in
    "" | *[!0-9A-Za-z.-]*) die "not a release version (expected 0.1.0-alpha.1, a leading v is fine, or nightly)" ;;
  esac
  printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
    || die "not a release version: $version (expected 0.1.0-alpha.1, a leading v is fine, or nightly)"
  tag="v$version"
fi

base="${LATTICE_INSTALL_BASE:-https://github.com/$REPO/releases/download/$tag}"
name="lattice-$version-$target"
archive="$name.tar.gz"

say "downloading $archive and SHA256SUMS ($tag)"
fetch "$base/$archive" -o "$tmp/$archive" \
  || die "could not download $base/$archive (does release $tag exist for $target?)"
fetch "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" \
  || die "could not download $base/SHA256SUMS"

expected=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "SHA256SUMS has no entry for $archive. Nothing was installed."
actual=$(sha256 "$tmp/$archive")
if [ "$expected" != "$actual" ]; then
  die "checksum mismatch for $archive (SHA256SUMS says $expected, the download is $actual). Nothing was installed."
fi
say "checksum ok"

if command -v gh >/dev/null 2>&1; then
  say "verifying the build provenance attestation with gh"
  gh attestation verify "$tmp/$archive" --repo "$REPO" >/dev/null \
    || die "gh could not verify the attestation of $archive. Nothing was installed. If gh is not signed in, run: gh auth login"
  say "attestation ok"
else
  say "gh is not installed: skipping the attestation check (checksum only). To verify later: gh attestation verify $archive --repo $REPO"
fi

# Extract only the two programs; the archive also holds LICENSE, NOTICE and README.md.
tar -xzf "$tmp/$archive" -C "$tmp" "$name/lattice" "$name/latticed" \
  || die "the archive does not contain $name/lattice and $name/latticed"

dest="$prefix/bin"
mkdir -p "$dest" || die "cannot create $dest (pass --prefix with a directory you own; this script never uses sudo)"
for b in lattice latticed; do
  cp "$tmp/$name/$b" "$dest/.$b.new.$$" || die "cannot write to $dest"
  chmod 755 "$dest/.$b.new.$$"
  mv -f "$dest/.$b.new.$$" "$dest/$b"
done

say "installed to $dest"
"$dest/lattice" --version
"$dest/latticed" --version

case ":${PATH:-}:" in
  *":$dest:"*) ;;
  *) say "$dest is not on your PATH; add it to your shell profile yourself, this script does not edit profiles" ;;
esac
