#!/usr/bin/env bash
# tests/bash/installer_setup_exit_code.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# `gpy setup` exits 5 when the rc file holds an incomplete GPY block: every
# file is installed, but the shell does not load GPY. Both installers treat
# that code as a warning (exit 0, the agent is still started, and the last
# line says GPY is not active); any other non-zero setup code still fails the
# install. Stub binaries stand in for gpy and gpy-agent.

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

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# A CLI stub: `setup --check` passes; `setup` exits $STUB_SETUP_RC.
write_cli_stub() {
    cat >"$1" <<'EOF'
#!/bin/sh
case "$1" in
    --version) echo "gpy 9.9.9"; exit 0 ;;
    setup)
        for arg in "$@"; do [ "$arg" = --check ] && exit 0; done
        echo "stub: gpy setup"
        exit "${STUB_SETUP_RC:-0}"
        ;;
esac
exit 0
EOF
    chmod +x "$1"
}

# An agent stub that records every call in $STUB_CALLS.
write_agent_stub() {
    cat >"$1" <<'EOF'
#!/bin/sh
[ "$1" = --version ] && { echo "gpy-agent 9.9.9"; exit 0; }
echo "gpy-agent $*" >>"$STUB_CALLS"
exit 0
EOF
    chmod +x "$1"
}

# Serves the stubs and the shell archive for install-oneline.sh.
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

# The release archive layout install.sh runs from.
PKG="$WORKDIR/pkg"
mkdir -p "$PKG/bin"
cp "$ROOT/install.sh" "$PKG/install.sh"
write_agent_stub "$PKG/bin/$AGENT_ASSET"
write_cli_stub "$PKG/bin/$CLI_ASSET"
for asset in "$AGENT_ASSET" "$CLI_ASSET"; do
    printf '%s  %s\n' "$(digest_of "$PKG/bin/$asset")" "$asset" >"$PKG/bin/$asset.sha256"
done

# What install-oneline.sh downloads.
STUBS="$WORKDIR/stubs"
FAKEBIN="$WORKDIR/fakebin"
mkdir -p "$STUBS" "$FAKEBIN"
write_agent_stub "$STUBS/agent"
write_cli_stub "$STUBS/cli"
shell_e2e_build_shell_archive "$ROOT" "$WORKDIR/gpy-shell-files.tar.gz" ||
    { echo "FAIL: could not build the shell archive"; exit 1; }
write_fake_curl "$FAKEBIN" "$STUBS/agent" "$STUBS/cli" "$WORKDIR/gpy-shell-files.tar.gz"

# run_installer NAME SETUP_RC: run installer NAME (install.sh or
# install-oneline.sh) in a fresh HOME with `gpy setup` exiting SETUP_RC. Sets
# STATUS, OUTPUT (stdout and stderr) and HOME_DIR.
run_installer() {
    HOME_DIR="$WORKDIR/home-$1-$2"
    mkdir -p "$HOME_DIR"
    : >"$HOME_DIR/calls.log"
    local log="$HOME_DIR/install.log"
    STATUS=0
    if [ "$1" = install.sh ]; then
        (cd "$PKG" && env HOME="$HOME_DIR" XDG_CONFIG_HOME="$HOME_DIR/.config" \
            XDG_DATA_HOME="$HOME_DIR/.local/share" XDG_CACHE_HOME="$HOME_DIR/.cache" \
            GPY_SHELL=bash STUB_SETUP_RC="$2" STUB_CALLS="$HOME_DIR/calls.log" \
            bash ./install.sh) </dev/null >"$log" 2>&1 || STATUS=$?
    else
        env HOME="$HOME_DIR" XDG_CONFIG_HOME="$HOME_DIR/.config" \
            XDG_DATA_HOME="$HOME_DIR/.local/share" XDG_CACHE_HOME="$HOME_DIR/.cache" \
            PATH="$FAKEBIN:$PATH" GPY_SHELL=bash GPY_VERSION=v9.9.9-test \
            STUB_SETUP_RC="$2" STUB_CALLS="$HOME_DIR/calls.log" \
            sh "$ROOT/install-oneline.sh" </dev/null >"$log" 2>&1 || STATUS=$?
    fi
    OUTPUT="$(cat "$log")"
}

for installer in install.sh install-oneline.sh; do
    run_installer "$installer" 5
    check "$installer: setup exit 5 is a warning, the install exits 0" 0 "$STATUS"
    if [ -x "$HOME_DIR/.local/bin/gpy" ] && [ -x "$HOME_DIR/.local/bin/gpy-agent" ]; then
        pass "$installer: binaries installed despite exit 5"
    else
        fail "$installer: binaries missing after exit 5: $OUTPUT"
    fi
    if grep -q '^gpy-agent start' "$HOME_DIR/calls.log"; then
        pass "$installer: the agent is still started after exit 5"
    else
        fail "$installer: the agent was not started after exit 5: $OUTPUT"
    fi
    last_line="$(printf '%s\n' "$OUTPUT" | sed '/^$/d' | tail -n 1)"
    case "$last_line" in
        *"GPY is NOT active"*"incomplete"*) pass "$installer: the last line says GPY is not active" ;;
        *) fail "$installer: last line does not say GPY is not active: [$last_line]" ;;
    esac
    case "$OUTPUT" in
        *"installed successfully"*) fail "$installer: claims success with an incomplete block: $OUTPUT" ;;
        *) pass "$installer: no success claim with an incomplete block" ;;
    esac

    run_installer "$installer" 3
    if [ "$STATUS" -ne 0 ]; then
        pass "$installer: setup exit 3 fails the install (exit $STATUS)"
    else
        fail "$installer: setup exit 3 did not fail the install: $OUTPUT"
    fi
    case "$OUTPUT" in
        *"GPY is NOT active"* | *"installed successfully"*)
            fail "$installer: a failed setup reached the closing summary: $OUTPUT" ;;
        *"gpy setup"*) pass "$installer: the failure names gpy setup" ;;
        *) fail "$installer: the failure does not name gpy setup: $OUTPUT" ;;
    esac
    if grep -q '^gpy-agent start' "$HOME_DIR/calls.log"; then
        fail "$installer: the agent was started after a failed setup"
    else
        pass "$installer: a failed setup stops before the agent start"
    fi
done

test_finish "installers treat gpy setup exit 5 as a warning and other codes as failures"
