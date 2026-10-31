#!/usr/bin/env bash
# tests/bash/install_oneline_failed_upgrade.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh must verify a downloaded binary BEFORE replacing the
# installed one; otherwise an unrunnable asset leaves a broken agent behind.
#
#   broken agent  installer exits non-zero; the old agent is byte-identical
#                 and still runs; no .gpy-agent.new.* is left
#   broken CLI    installer exits non-zero (the CLI is required: it runs
#                 `gpy setup`); both the old gpy and the old agent are
#                 byte-identical and still run; no .gpy*.new.* is left
#   good upgrade  both binaries are replaced by the new ones

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux | darwin) ;;
    *) test_skip "install-oneline.sh does not support $(uname -s)" ;;
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

# A "working" stub CLI must run the real `gpy setup` (the installer delegates shell wiring).
REAL_GPY="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$REAL_GPY" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy) || {
        echo "FAIL: could not build the gpy binary"
        exit 1
    }
fi

write_stub() {
    case "$2" in
        working) printf '#!/bin/sh\necho "%s"\nexit 0\n' "$3" >"$1" ;;
        broken) printf '#!/bin/sh\nexit 1\n' >"$1" ;;
    esac
    chmod +x "$1"
}

write_cli_stub() {
    case "$2" in
        working)
            # shellcheck disable=SC2016  # the stub's own $1 and $@ must stay literal
            printf '#!/bin/sh\nif [ "$1" = setup ]; then exec "%s" "$@"; fi\necho "%s"\nexit 0\n' "$REAL_GPY" "$3" >"$1"
            chmod +x "$1"
            ;;
        *) write_stub "$@" ;;
    esac
}

shell_e2e_init "$ROOT"
shell_e2e_build_shell_archive "$ROOT" "$SHELL_E2E_ROOT/gpy-shell-files.tar.gz" ||
    { echo "FAIL: could not build the shell archive"; exit 1; }

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
    write_cli_stub "$stubs/cli" "$new_cli" "gpy 9.9.9"
    write_fake_curl "$fakebin" "$stubs/agent" "$stubs/cli" "$SHELL_E2E_ROOT/gpy-shell-files.tar.gz"
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

run_scenario broken_cli working broken
if [ "$rc" -ne 0 ]; then
    pass "broken CLI: installer exited non-zero ($rc)"
else
    fail "broken CLI: installer exited 0"
fi
if [ "$(digest_of "$bindir/gpy")" = "$old_cli_sum" ] \
    && [ "$("$bindir/gpy" --version 2>/dev/null)" = "gpy 0.0.1" ]; then
    pass "broken CLI: the old gpy is byte-identical and still runs"
else
    fail "broken CLI: the installed gpy was replaced"
fi
if [ "$(digest_of "$bindir/gpy-agent")" = "$old_agent_sum" ] \
    && [ "$("$bindir/gpy-agent" --version 2>/dev/null)" = "gpy-agent 0.0.1" ]; then
    pass "broken CLI: the old agent is byte-identical and still runs"
else
    fail "broken CLI: the agent was replaced before the CLI failed"
fi
if [ -z "$(find "$bindir" -name '*.backup.*' 2>/dev/null)" ]; then
    pass "broken CLI: no backup was created"
else
    fail "broken CLI: a backup was created"
fi
if no_staged_files; then
    pass "broken CLI: no staged file left behind"
else
    fail "broken CLI: staged file left: $(find "$bindir" -name '.gpy*.new.*')"
fi
if grep -q 'not functional' "$log"; then
    pass "broken CLI: the error says the CLI is not functional"
else
    fail "broken CLI: no 'not functional' message in the log"
fi

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

test_finish "a failed binary check leaves the previously installed agent and gpy untouched"
