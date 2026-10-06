#!/usr/bin/env bash
# tests/bash/install_oneline_failed_upgrade.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh must verify a downloaded binary BEFORE it replaces the
# installed one (#806). A checksum-valid asset that cannot run on the host
# (needs a newer glibc, wrong libc, wrong userland) used to be moved over a
# working ~/.local/bin/gpy-agent first and checked afterwards, leaving a
# broken agent behind and the old one only as a backup nobody restored.
#
# Each scenario runs the installer in its own sandboxed HOME behind a fake
# `curl` serving stub assets (with matching `.sha256` sidecars), with an
# older working agent and CLI pre-installed:
#
#   broken agent  installer exits non-zero; the old agent is byte-identical
#                 and still runs; no .gpy-agent.new.* is left
#   broken CLI    installer exits 0 with a warning; the old gpy is
#                 byte-identical and still runs; the new agent is installed;
#                 no .gpy.new.* is left
#   good upgrade  both binaries are replaced by the new ones

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux | darwin) ;;
    *) test_skip "install-oneline.sh does not support $(uname -s)" ;;
esac

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# A `curl` that serves any gpy-agent-* / gpy-* release asset from $AGENT_SRC /
# $CLI_SRC and every raw.githubusercontent.com shell file from this checkout.
write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
ROOT="$ROOT"
AGENT_SRC="$2"
CLI_SRC="$3"
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
        */releases/download/*/gpy-agent-*) echo "$AGENT_SRC" ;;
        */releases/download/*/gpy-*) echo "$CLI_SRC" ;;
        */raw.githubusercontent.com/*) echo "$ROOT/${1#*/raw.githubusercontent.com/*/*/*/}" ;;
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

# `write_stub FILE working|broken VERSION`
write_stub() {
    case "$2" in
        working) printf '#!/bin/sh\necho "%s"\nexit 0\n' "$3" >"$1" ;;
        broken) printf '#!/bin/sh\nexit 1\n' >"$1" ;;
    esac
    chmod +x "$1"
}

shell_e2e_init "$ROOT"

# `run_scenario NAME NEW_AGENT NEW_CLI` -- sets rc, bindir, log.
run_scenario() {
    name="$1" new_agent="$2" new_cli="$3"
    echo "=== $name ==="
    home="$SHELL_E2E_ROOT/$name/home"
    stubs="$SHELL_E2E_ROOT/$name/stubs"
    fakebin="$SHELL_E2E_ROOT/$name/fakebin"
    bindir="$home/.local/bin"
    log="$SHELL_E2E_ROOT/$name/install.log"
    mkdir -p "$home" "$stubs" "$fakebin" "$bindir"

    write_stub "$stubs/agent" "$new_agent" "gpy-agent 9.9.9"
    write_stub "$stubs/cli" "$new_cli" "gpy 9.9.9"
    write_fake_curl "$fakebin" "$stubs/agent" "$stubs/cli"
    write_stub "$bindir/gpy-agent" working "gpy-agent 0.0.1"
    write_stub "$bindir/gpy" working "gpy 0.0.1"
    old_agent_sum="$(digest_of "$bindir/gpy-agent")"
    old_cli_sum="$(digest_of "$bindir/gpy")"
    printf '# user rc before gpy\n' >"$home/.bashrc"

    env HOME="$home" PATH="$fakebin:$bindir:$PATH" GPY_SHELL=bash GPY_VERSION=v9.9.9-test \
        sh "$ROOT/install-oneline.sh" </dev/null >"$log" 2>&1
    rc=$?
}

no_staged_files() {
    # `-name` patterns are quoted, so a missing match prints nothing.
    [ -z "$(find "$bindir" -name '.gpy*.new.*' 2>/dev/null)" ]
}

# --- broken agent: the old agent survives, the installer fails -------------
run_scenario broken_agent broken working
if [ "$rc" -ne 0 ]; then
    pass "broken agent: installer exited non-zero ($rc)"
else
    fail "broken agent: installer exited 0"
fi
if [ "$(digest_of "$bindir/gpy-agent")" = "$old_agent_sum" ]; then
    pass "broken agent: the old agent is byte-identical"
else
    fail "broken agent: the installed agent was replaced"
fi
if [ "$("$bindir/gpy-agent" --version 2>/dev/null)" = "gpy-agent 0.0.1" ]; then
    pass "broken agent: the old agent still runs"
else
    fail "broken agent: the installed agent no longer prints gpy-agent 0.0.1"
fi
if no_staged_files; then
    pass "broken agent: no staged file left behind"
else
    fail "broken agent: staged file left: $(find "$bindir" -name '.gpy*.new.*')"
fi
if grep -q 'left untouched' "$log"; then
    pass "broken agent: the error says the existing install was left untouched"
else
    fail "broken agent: no 'left untouched' message: $(cat "$log")"
fi

# --- broken CLI: the old CLI survives, the installer continues -------------
run_scenario broken_cli working broken
if [ "$rc" -eq 0 ]; then
    pass "broken CLI: installer exited 0"
else
    fail "broken CLI: installer exited $rc: $(cat "$log")"
fi
if [ "$(digest_of "$bindir/gpy")" = "$old_cli_sum" ] \
    && [ "$("$bindir/gpy" --version 2>/dev/null)" = "gpy 0.0.1" ]; then
    pass "broken CLI: the old gpy is byte-identical and still runs"
else
    fail "broken CLI: the installed gpy was replaced"
fi
if [ "$("$bindir/gpy-agent" --version 2>/dev/null)" = "gpy-agent 9.9.9" ]; then
    pass "broken CLI: the new agent was still installed"
else
    fail "broken CLI: the agent was not upgraded"
fi
if no_staged_files; then
    pass "broken CLI: no staged file left behind"
else
    fail "broken CLI: staged file left: $(find "$bindir" -name '.gpy*.new.*')"
fi
if grep -q 'not functional' "$log"; then
    pass "broken CLI: the installer warned"
else
    fail "broken CLI: no warning in the log"
fi

# --- good upgrade: unchanged behaviour ------------------------------------
run_scenario good_upgrade working working
if [ "$rc" -eq 0 ]; then
    pass "good upgrade: installer exited 0"
else
    fail "good upgrade: installer exited $rc: $(cat "$log")"
fi
if [ "$("$bindir/gpy-agent" --version 2>/dev/null)" = "gpy-agent 9.9.9" ] \
    && [ "$("$bindir/gpy" --version 2>/dev/null)" = "gpy 9.9.9" ]; then
    pass "good upgrade: both binaries were replaced"
else
    fail "good upgrade: binaries not upgraded"
fi
if [ -x "$bindir/gpy-agent" ] && no_staged_files; then
    pass "good upgrade: no staged file left behind"
else
    fail "good upgrade: staged file left or agent missing"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAIL: $failures check(s) failed"
    exit 1
fi
echo "PASS: a failed binary check leaves the previously installed agent and gpy untouched"
