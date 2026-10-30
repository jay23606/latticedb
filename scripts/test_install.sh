#!/bin/sh
# Offline tests for scripts/install.sh. No network: the script is pointed at a fake release
# directory through file:// URLs, uname and gh are replaced by shims, and the PATH holds only the
# tools the installer is allowed to use. Run: sh scripts/test_install.sh
#
# Each test function is run in its own subshell and temporary directory. The expectations are
# written by hand (the file names and the messages), not taken from the script's own output.

set -u

here=$(cd "$(dirname "$0")" && pwd)
script="$here/install.sh"
root=$(mktemp -d "${TMPDIR:-/tmp}/lattice-install-test.XXXXXX") || exit 1
trap 'rm -rf "$root"' EXIT HUP INT TERM

VERSION="0.1.0-alpha.1"

# A PATH of symlinks to exactly the tools install.sh uses, so a gh on the developer's machine
# cannot leak into a test and a missing gh is a real "gh is not installed".
mk_toolpath() { # dir
  mkdir -p "$1"
  for t in curl tar awk grep sed head mktemp rm mkdir cp chmod mv cat sha256sum shasum; do
    p=$(command -v "$t" 2>/dev/null) || continue
    case "$p" in /*) ln -s "$p" "$1/$t" ;; esac
  done
  cat > "$1/uname" <<'SHIM'
#!/bin/sh
case "$1" in
  -s) printf '%s\n' "${FAKE_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${FAKE_UNAME_M:-x86_64}" ;;
  *) exit 2 ;;
esac
SHIM
  chmod +x "$1/uname"
}

sum_of() { # file
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

# make_release DIR TARGET: lays out what the package workflow publishes for one target:
# lattice-<version>-<target>.tar.gz holding a directory of the same name with lattice, latticed,
# LICENSE, NOTICE and README.md, plus a SHA256SUMS line "<hash>  <file>".
make_release() {
  rel="$1"; target="$2"; name="lattice-$VERSION-$target"
  mkdir -p "$rel/stage/$name"
  for b in lattice latticed; do
    printf '#!/bin/sh\necho "%s %s"\n' "$b" "$VERSION" > "$rel/stage/$name/$b"
    chmod +x "$rel/stage/$name/$b"
  done
  for f in LICENSE NOTICE README.md; do echo "$f" > "$rel/stage/$name/$f"; done
  tar -czf "$rel/$name.tar.gz" -C "$rel/stage" "$name"
  printf '%s  %s\n' "$(sum_of "$rel/$name.tar.gz")" "$name.tar.gz" > "$rel/SHA256SUMS"
  rm -rf "$rel/stage"
}

# run_install ARGS...: runs the script with the restricted PATH; output in $out, status in $status.
run_install() {
  out=$(PATH="$tp" HOME="$tmp/home" LATTICE_INSTALL_BASE="file://$tmp/rel" /bin/sh "$script" "$@" 2>&1)
  status=$?
}

fail() { echo "  assertion failed: $*" >&2; exit 1; }
expect_status() { [ "$status" -eq "$1" ] || fail "exit status $status, wanted $1; output: $out"; }
expect_nonzero() { [ "$status" -ne 0 ] || fail "wanted a failure but the script succeeded; output: $out"; }
expect_out() { case "$out" in *"$1"*) ;; *) fail "output lacks '$1': $out" ;; esac; }
expect_no_install() { [ ! -e "$1/bin/lattice" ] && [ ! -e "$1/bin/latticed" ] || fail "binaries were installed in $1"; }

setup() { # also called at the start of every test, inside its subshell
  tmp=$(mktemp -d "$root/t.XXXXXX")
  tp="$tmp/tools"; mk_toolpath "$tp"
  mkdir -p "$tmp/rel" "$tmp/home"
}

install_puts_both_programs_in_prefix_bin_for_each_platform() {
  for pair in "Linux x86_64 x86_64-unknown-linux-gnu" "Linux aarch64 aarch64-unknown-linux-gnu" \
              "Darwin arm64 aarch64-apple-darwin"; do
    # shellcheck disable=SC2086 # the pair is split into words on purpose
    set -- $pair
    setup
    make_release "$tmp/rel" "$3"
    FAKE_UNAME_S="$1" FAKE_UNAME_M="$2" run_install --prefix "$tmp/p" "$VERSION"
    expect_status 0
    expect_out "lattice $VERSION"
    expect_out "latticed $VERSION"
    [ -x "$tmp/p/bin/lattice" ] && [ -x "$tmp/p/bin/latticed" ] || fail "not installed for $3"
    [ ! -e "$tmp/p/bin/LICENSE" ] || fail "extra files were installed"
  done
}

install_refuses_when_the_checksum_does_not_match() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  f="$tmp/rel/lattice-$VERSION-x86_64-unknown-linux-gnu.tar.gz"
  # Flip one bit of the byte at offset 40 after SHA256SUMS was written.
  old=$(dd if="$f" bs=1 skip=40 count=1 2>/dev/null | od -An -tu1 | tr -d ' ')
  new=$((old ^ 1))
  printf '%b' "\\0$(printf '%o' "$new")" | dd of="$f" bs=1 seek=40 count=1 conv=notrunc 2>/dev/null
  [ "$(sum_of "$f")" != "$(awk '{print $1}' "$tmp/rel/SHA256SUMS")" ] || fail "test setup did not change the archive"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_nonzero
  expect_out "checksum mismatch"
  expect_no_install "$tmp/p"
}

install_refuses_an_unknown_platform_with_a_clear_message() {
  for pair in "Linux riscv64" "Darwin x86_64" "FreeBSD amd64" "MINGW64_NT-10.0 x86_64"; do
    # shellcheck disable=SC2086 # the pair is split into words on purpose
    set -- $pair
    setup
    make_release "$tmp/rel" x86_64-unknown-linux-gnu
    FAKE_UNAME_S="$1" FAKE_UNAME_M="$2" run_install --prefix "$tmp/p" "$VERSION"
    expect_nonzero
    expect_out "unsupported platform $1/$2"
    expect_out "macOS arm64"
    expect_out "Linux x86_64"
    expect_out "Linux arm64"
    expect_no_install "$tmp/p"
  done
}

install_refuses_when_sha256sums_has_no_entry_for_the_archive() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  echo "0000000000000000000000000000000000000000000000000000000000000000  other.tar.gz" > "$tmp/rel/SHA256SUMS"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_nonzero
  expect_out "no entry"
  expect_no_install "$tmp/p"
}

install_skips_attestation_and_says_so_when_gh_is_absent() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_status 0
  expect_out "gh is not installed"
}

install_verifies_the_attestation_with_gh_when_present() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  printf '#!/bin/sh\necho "$@" > "%s/gh.args"\nexit 0\n' "$tmp" > "$tp/gh"; chmod +x "$tp/gh"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_status 0
  args=$(cat "$tmp/gh.args")
  case "$args" in "attestation verify "*"lattice-$VERSION-x86_64-unknown-linux-gnu.tar.gz --repo moncifem/latticedb") ;;
    *) fail "unexpected gh arguments: $args" ;; esac
}

install_refuses_when_the_attestation_does_not_verify() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  printf '#!/bin/sh\necho "no attestation" >&2\nexit 1\n' > "$tp/gh"; chmod +x "$tp/gh"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_nonzero
  expect_out "could not verify the attestation"
  expect_no_install "$tmp/p"
}

install_defaults_to_the_newest_pre_release_tag() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  cat > "$tmp/api.json" <<'JSON'
[
  { "tag_name": "nightly", "prerelease": true },
  { "tag_name": "v0.1.0-alpha.1", "prerelease": true },
  { "tag_name": "v0.1.0-alpha.0", "prerelease": true }
]
JSON
  LATTICE_INSTALL_API="file://$tmp/api.json" FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p"
  expect_status 0
  expect_out "newest pre-release: v0.1.0-alpha.1"
  [ -x "$tmp/p/bin/lattice" ] || fail "not installed"
}

install_fails_clearly_when_there_is_no_pre_release_tag() {
  setup
  cat > "$tmp/api.json" <<'JSON'
[ { "tag_name": "nightly", "prerelease": true }, { "tag_name": "v1.0.0", "prerelease": false } ]
JSON
  LATTICE_INSTALL_API="file://$tmp/api.json" FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p"
  expect_nonzero
  expect_out "no pre-release tag"
  expect_no_install "$tmp/p"
}

install_fails_clearly_when_the_release_list_cannot_be_read() {
  setup
  LATTICE_INSTALL_API="file://$tmp/missing.json" FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p"
  expect_nonzero
  expect_out "could not read the release list"
}

install_rejects_a_version_that_is_not_a_release_version() {
  setup
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "../../etc/passwd"
  expect_nonzero
  expect_out "not a release version"
}

install_rejects_a_version_with_a_newline() {
  setup
  nl='
'
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "0.1.0${nl}9.9.9"
  expect_nonzero
  expect_out "not a release version"
  expect_no_install "$tmp/p"
}

install_ignores_sha256sums_entries_that_only_contain_the_archive_name() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  a="lattice-$VERSION-x86_64-unknown-linux-gnu.tar.gz"
  h=$(awk '{print $1}' "$tmp/rel/SHA256SUMS")
  printf '%s  x%s\n%s  %s.sig\n' "$h" "$a" "$h" "$a" > "$tmp/rel/SHA256SUMS"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_nonzero
  expect_out "no entry"
  expect_no_install "$tmp/p"
}

install_uses_the_first_sha256sums_line_for_the_archive() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  a="lattice-$VERSION-x86_64-unknown-linux-gnu.tar.gz"
  good=$(awk '{print $1}' "$tmp/rel/SHA256SUMS")
  bad=0000000000000000000000000000000000000000000000000000000000000000
  # A bad line first wins and refuses; a good line first wins and installs.
  printf '%s  %s\n%s  %s\n' "$bad" "$a" "$good" "$a" > "$tmp/rel/SHA256SUMS"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_nonzero
  expect_out "checksum mismatch"
  printf '%s  %s\n%s  %s\n' "$good" "$a" "$bad" "$a" > "$tmp/rel/SHA256SUMS"
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install --prefix "$tmp/p" "$VERSION"
  expect_status 0
}

install_defaults_to_home_local_bin_and_leaves_profiles_alone() {
  setup
  make_release "$tmp/rel" x86_64-unknown-linux-gnu
  FAKE_UNAME_S=Linux FAKE_UNAME_M=x86_64 run_install "v$VERSION"
  expect_status 0
  [ -x "$tmp/home/.local/bin/lattice" ] || fail "default location not used"
  found=$(find "$tmp/home" -type f ! -path "$tmp/home/.local/bin/*") || true
  [ -z "$found" ] || fail "script wrote other files: $found"
}

failed=0
for t in \
  install_puts_both_programs_in_prefix_bin_for_each_platform \
  install_refuses_when_the_checksum_does_not_match \
  install_refuses_an_unknown_platform_with_a_clear_message \
  install_refuses_when_sha256sums_has_no_entry_for_the_archive \
  install_skips_attestation_and_says_so_when_gh_is_absent \
  install_verifies_the_attestation_with_gh_when_present \
  install_refuses_when_the_attestation_does_not_verify \
  install_defaults_to_the_newest_pre_release_tag \
  install_fails_clearly_when_there_is_no_pre_release_tag \
  install_fails_clearly_when_the_release_list_cannot_be_read \
  install_rejects_a_version_that_is_not_a_release_version \
  install_rejects_a_version_with_a_newline \
  install_ignores_sha256sums_entries_that_only_contain_the_archive_name \
  install_uses_the_first_sha256sums_line_for_the_archive \
  install_defaults_to_home_local_bin_and_leaves_profiles_alone
do
  if ( "$t" ); then echo "ok   $t"; else echo "FAIL $t"; failed=$((failed + 1)); fi
done
[ "$failed" -eq 0 ] || { echo "$failed test(s) failed" >&2; exit 1; }
echo "all install tests passed"
