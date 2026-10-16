#!/usr/bin/env bash
# shellcheck disable=SC2016
# tests/bash/shell_e2e_teardown_guard.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shell_e2e.sh must stop the agent and remove the sandbox when a child test fails,
# exits, is interrupted or is killed. Only the child's own socket PID is checked or killed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command lsof
test_require_command python3

# Short path: unix socket paths are capped near 104 bytes on macOS.
scratch="$(mktemp -d /tmp/gpy-guard.XXXXXX)" || exit 1
child_pid=""
agent_pid=""
cleanup() {
    [[ -n "$child_pid" ]] && kill -9 "$child_pid" 2>/dev/null
    [[ -n "$agent_pid" ]] && kill -9 "$agent_pid" 2>/dev/null
    rm -rf "$scratch"
}
trap cleanup EXIT

# The child test, written for bash (source directly) or zsh (sh emulation).
write_child() {
    local shell="$1" file="$2"
    {
        if [[ "$shell" == zsh ]]; then
            printf '%s\n' 'ROOT=$1; out=$2; mode=$3' \
                'emulate sh -c '"'"'. "$ROOT/tests/lib/shell_e2e.sh"'"'"
        else
            printf '%s\n' 'ROOT=$1; out=$2; mode=$3' '. "$ROOT/tests/lib/shell_e2e.sh"'
        fi
        printf '%s\n' \
            'shell_e2e_init "$ROOT"' \
            'printf "%s\n" "$SHELL_E2E_ROOT" >"$out/root"' \
            'shell_e2e_start_agent || exit 9' \
            'lsof -t "$GPY_AGENT_SOCKET_PATH" | head -n 1 >"$out/agent_pid"' \
            ': >"$out/ready"' \
            'shell_e2e_poll 30 test -e "$out/go" || exit 8' \
            'case "$mode" in' \
            '    fail) fail "deliberate assertion failure"; test_finish "unreachable" ;;' \
            '    exit3) exit 3 ;;' \
            '    normal) exit 0 ;;' \
            'esac'
    } >"$file"
}

run_case() {
    local shell="$1" mode="$2" sig="${3:-}" want_rc="$4"
    local label="$shell/$mode${sig:+/SIG$sig}" out="$scratch/$shell-$mode${sig:+-$sig}"
    mkdir -p "$out"
    write_child "$shell" "$out/child"
    # A background job of a non-interactive shell starts with SIGINT ignored,
    # which no trap can undo; reset it as a terminal's Ctrl-C would find it.
    TMPDIR="$scratch" python3 -c 'import os, signal, sys
signal.signal(signal.SIGINT, signal.SIG_DFL)
os.execvp(sys.argv[1], sys.argv[1:])' "$shell" "$out/child" "$ROOT" "$out" "$mode" >"$out/log" 2>&1 &
    child_pid=$!

    if ! shell_e2e_poll 60 test -e "$out/ready"; then
        fail "$label: the child never became ready"
        cat "$out/log"
        kill -9 "$child_pid" 2>/dev/null
        child_pid=""
        return
    fi
    agent_pid="$(cat "$out/agent_pid" 2>/dev/null)"
    sandbox="$(cat "$out/root")"
    if [[ -z "$agent_pid" ]] || ! kill -0 "$agent_pid" 2>/dev/null; then
        fail "$label: no live agent to tear down (pid '${agent_pid}')"
        return
    fi

    # The child holds at `ready` until released, so the agent PID read above
    # is its live agent; a signal case is never released and is signalled.
    if [[ -n "$sig" ]]; then
        kill "-$sig" "$child_pid"
    else
        : >"$out/go"
    fi
    wait "$child_pid"
    local rc=$?
    child_pid=""

    agent_gone() { ! kill -0 "$agent_pid" 2>/dev/null; }
    if ! shell_e2e_poll 10 agent_gone; then
        fail "$label: the agent (pid $agent_pid) outlived the test"
        kill -9 "$agent_pid" 2>/dev/null
    else
        pass "$label: the agent is stopped"
    fi
    if [[ -e "$sandbox" ]]; then
        fail "$label: the sandbox $sandbox was not removed"
    else
        pass "$label: the sandbox is removed"
    fi
    if [[ "$rc" -eq "$want_rc" ]]; then
        pass "$label: exit status $rc preserved"
    else
        fail "$label: exit status $rc, want $want_rc"
    fi
    agent_pid=""
}

shells="bash"
command -v zsh >/dev/null 2>&1 && shells="bash zsh"
for shell in $shells; do
    run_case "$shell" normal "" 0
    run_case "$shell" fail "" 1
    run_case "$shell" exit3 "" 3
    run_case "$shell" hold TERM 143
    run_case "$shell" hold INT 130
done

test_finish "an exiting, failing, interrupted or terminated test leaves no agent or sandbox"
