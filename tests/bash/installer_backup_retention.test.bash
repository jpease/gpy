#!/usr/bin/env bash
# tests/bash/installer_backup_retention.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Both installers keep at most ONE backup per binary, and it must be the newest.
# Runs are a second apart so backup stamps differ.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
            aarch64 | arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    darwin)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
            arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    *) test_skip "the installers do not support $(uname -s)" ;;
esac

digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
AGENT_SRC="$2"
CLI_SRC="$3"
SHELL_ARCHIVE="$4"
EOF
    cat >>"$1/curl" <<'EOF'
out=""; url=""; prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then out="$arg"; else case "$arg" in -*) ;; *) url="$arg" ;; esac; fi
    prev="$arg"
done
digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
source_for() {
    case "$1" in
        */releases/download/*/gpy-shell-files.tar.gz) echo "$SHELL_ARCHIVE" ;;
        */releases/download/*/gpy-agent-*) echo "$AGENT_SRC" ;;
        */releases/download/*/gpy-*) echo "$CLI_SRC" ;;
        *) echo "" ;;
    esac
}
case "$url" in
    *.sha256)
        src="$(source_for "${url%.sha256}")"
        [ -n "$src" ] && [ -f "$src" ] || exit 22
        base="${url##*/}"
        printf '%s  %s\n' "$(digest_of "$src")" "${base%.sha256}"
        exit 0
        ;;
esac
src="$(source_for "$url")"
[ -n "$src" ] && [ -f "$src" ] || exit 22
if [ -n "$out" ]; then
    mkdir -p "$(dirname "$out")"
    cp "$src" "$out"
else
    cat "$src"
fi
exit 0
EOF
    chmod +x "$1/curl"
}

write_stub() {
    printf '#!/bin/sh\necho "%s"\nexit 0\n' "$2" >"$1"
    chmod +x "$1"
}

shell_e2e_init "$ROOT"

check_backups() {
    name="$1" bindir="$2"
    for bin in gpy-agent gpy; do
        count="$(find "$bindir" -maxdepth 1 -name "$bin.backup.*" | wc -l | tr -d ' ')"
        if [ "$count" -eq 1 ]; then
            pass "$name: exactly one $bin backup after 3 runs"
        else
            fail "$name: $count $bin backups after 3 runs: $(find "$bindir" -maxdepth 1 -name "$bin.backup.*")"
            continue
        fi
        kept="$(find "$bindir" -maxdepth 1 -name "$bin.backup.*")"
        if [ "$("$kept" --version 2>/dev/null)" = "$bin 9.9.2" ]; then
            pass "$name: the surviving $bin backup is the newest (9.9.2)"
        else
            fail "$name: the surviving $bin backup is not the 9.9.2 binary"
        fi
    done
}

home="$SHELL_E2E_ROOT/oneline/home"
stubs="$SHELL_E2E_ROOT/oneline/stubs"
fakebin="$SHELL_E2E_ROOT/oneline/fakebin"
bindir="$home/.local/bin"
mkdir -p "$home" "$stubs" "$fakebin" "$bindir"
shell_e2e_build_shell_archive "$ROOT" "$SHELL_E2E_ROOT/oneline/gpy-shell-files.tar.gz" ||
    { echo "FAIL: could not build the shell archive"; exit 1; }
write_fake_curl "$fakebin" "$stubs/agent" "$stubs/cli" "$SHELL_E2E_ROOT/oneline/gpy-shell-files.tar.gz"
printf '# user rc before gpy\n' >"$home/.bashrc"
for n in 1 2 3; do
    write_stub "$stubs/agent" "gpy-agent 9.9.$n"
    write_stub "$stubs/cli" "gpy 9.9.$n"
    env HOME="$home" PATH="$fakebin:$bindir:$PATH" GPY_SHELL=bash GPY_VERSION=v9.9.9-test \
        sh "$ROOT/install-oneline.sh" </dev/null >"$SHELL_E2E_ROOT/oneline/run$n.log" 2>&1
    rc=$?
    [ "$rc" -eq 0 ] || fail "install-oneline.sh run $n exited $rc: $(cat "$SHELL_E2E_ROOT/oneline/run$n.log")"
    [ "$n" -eq 3 ] || sleep 1 # backup stamps have one-second resolution
done
check_backups install-oneline.sh "$bindir"

home="$SHELL_E2E_ROOT/pkg/home"
pkg="$SHELL_E2E_ROOT/pkg/payload"
bindir="$home/.local/bin"
mkdir -p "$home" "$pkg/bin"
cp -R "$ROOT/fish" "$pkg/fish"
cp "$ROOT/install.sh" "$pkg/install.sh"
for n in 1 2 3; do
    write_stub "$pkg/bin/$AGENT_ASSET" "gpy-agent 9.9.$n"
    write_stub "$pkg/bin/$CLI_ASSET" "gpy 9.9.$n"
    for asset in "$AGENT_ASSET" "$CLI_ASSET"; do
        printf '%s  %s\n' "$(digest_of "$pkg/bin/$asset")" "$asset" >"$pkg/bin/$asset.sha256"
    done
    (cd "$pkg" && env HOME="$home" XDG_CONFIG_HOME="$home/.config" GPY_SHELL=fish bash ./install.sh) \
        >"$SHELL_E2E_ROOT/pkg/run$n.log" 2>&1
    rc=$?
    [ "$rc" -eq 0 ] || fail "install.sh run $n exited $rc: $(cat "$SHELL_E2E_ROOT/pkg/run$n.log")"
    [ "$n" -eq 3 ] || sleep 1 # backup stamps have one-second resolution
done
check_backups install.sh "$bindir"

test_finish "each installer keeps exactly one backup per binary across repeated runs"
