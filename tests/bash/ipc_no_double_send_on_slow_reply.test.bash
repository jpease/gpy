#!/usr/bin/env bash
# tests/bash/ipc_no_double_send_on_slow_reply.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A late-answering agent: (a) the request is sent exactly once, (b) status 2
# with empty output so no caller forks `gpy-agent oneshot`. Only an unreachable
# agent (status 1) falls back. Each available transport is tested alone.
# (c) A silent connection opens the late window: further requests to that
# socket are status 2 without connecting, until it closes or the shell
# re-registers; a slow path-bearing op (git) never opens it.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command python3 "python3 not installed, cannot simulate a slow-to-reply agent"

# shellcheck source=bash/core/constants.bash
. bash/core/constants.bash
# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-slow-reply.XXXXXX")"
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT
# The late-window stamp lives in the runtime root; the shim PATHs have no mkdir.
export XDG_RUNTIME_DIR="$tmp/run"
mkdir -p "$XDG_RUNTIME_DIR/gpy/refresh-throttle"

real_path="$PATH"
make_shim() {
    local dir="$tmp/shim-$1" tool src
    shift
    mkdir -p "$dir"
    for tool in "$@"; do
        src="$(command -v "$tool")" || return 1
        ln -sf "$src" "$dir/$tool"
    done
    printf '%s' "$dir"
}

# Replies after DELAY seconds, counting connections in $tmp/connections. A new
# listener is a new agent, so no late window carries over to it.
start_listener() {
    rm -f "$tmp/slow.sock" "$tmp/connections"
    __gpy_agent_late_clear
    shell_e2e_stub_listener "$tmp/slow.sock" reply --reply '{"status":"ok"}' \
        --delay "$1" --lifetime 6 --connections "$tmp/connections"
}

check_transport() {
    local label="$1" use_path="$2" delay="$3" out status connections
    start_listener "$delay"
    export GPY_AGENT_SOCKET_PATH="$tmp/slow.sock" GPY_IPC_TIMEOUT_MS=150
    PATH="$use_path"
    out="$(__gpy_send_json '{"op":"ping"}')"
    status=$?
    PATH="$real_path"
    # Leave a window in which a resend would open a second connection.
    sleep 0.3
    shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
    connections="$(shell_e2e_count_lines "$tmp/connections")"

    if [ "$connections" -eq 1 ]; then
        pass "$label: one logical request is one connection"
    else
        fail "$label: expected exactly one connection, got $connections"
    fi
    if [ "$status" -eq 2 ] && [ -z "$out" ]; then
        pass "$label: a connected-but-late agent is status 2 with no output"
    else
        fail "$label: status $status, output '$out' (want status 2, empty)"
    fi
}

ran=0
if command -v socat >/dev/null 2>&1; then
    ran=$((ran + 1))
    check_transport "socat" "$(make_shim socat socat)" 0.5
fi
if command -v nc >/dev/null 2>&1; then
    nc_shim="$(make_shim nc nc)"
    PATH="$nc_shim"
    __gpy_nc_supports_unix=""
    __gpy_nc_probe_capabilities
    PATH="$real_path"
    if [ "$__gpy_nc_supports_unix" = 1 ]; then
        ran=$((ran + 1))
        # nc's -w is whole seconds, so the reply has to be later than 1s.
        check_transport "nc (no timeout)" "$nc_shim" 1.6
        if command -v timeout >/dev/null 2>&1; then
            ran=$((ran + 1))
            check_transport "nc (timeout wrapper)" "$(make_shim nct nc timeout)" 0.5
        fi
    fi
fi
[ "$ran" -gt 0 ] || test_skip "neither socat nor an nc with -U is installed"

# (c) On the real PATH, whichever transport __gpy_send_json picks. The reply
# comes after nc -w 1, so every connected request is late.
start_listener 1.6
export GPY_AGENT_SOCKET_PATH="$tmp/slow.sock" GPY_IPC_TIMEOUT_MS=150
statuses=""
for request in '{"op":"git","cwd":"/"}' '{"op":"ping"}' '{"op":"ping"}' '{"op":"clock"}'; do
    __gpy_send_json "$request" >/dev/null
    statuses+="$? "
done
__gpy_agent_late_clear
__gpy_send_json '{"op":"ping"}' >/dev/null
statuses+="$?"
sleep 0.3
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
check "every request in and around the late window is status 2" "2 2 2 2 2" "$statuses"
# git and the first ping connect; the ping and clock inside the window (at
# least four seconds) do not; the ping after it was closed connects again.
check "the late window skips the connection until it is closed" "3" "$(shell_e2e_count_lines "$tmp/connections")"

# __gpy_request never forks oneshot for a late agent, but does for an unreachable one.
mkdir -p "$tmp/bin" "$tmp/cache"
oneshot_log="$tmp/oneshot.log"
# shellcheck disable=SC2016 # $ is for the stub script, not this shell
printf '%s\n' '#!/bin/sh' 'echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"' 'echo ONESHOT_OUTPUT' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"
request_path="$tmp/bin:$real_path"

run_request() {
    : >"$oneshot_log"
    (
        export GPY_AGENT_SOCKET_PATH="$1" GPY_IPC_TIMEOUT_MS=150 \
            GPY_TEST_ONESHOT_LOG="$oneshot_log" XDG_CACHE_HOME="$tmp/cache" PATH="$request_path"
        hash -r
        __gpy_request git "$tmp" ansi true
    )
}

start_listener 0.6
out="$(run_request "$tmp/slow.sock")"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
if [ -z "$out" ] && [ ! -s "$oneshot_log" ]; then
    pass "__gpy_request omits the segment for a late agent and forks no oneshot"
else
    fail "late agent: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

out="$(run_request "$tmp/missing.sock")"
if [ "$out" = ONESHOT_OUTPUT ] && [ -s "$oneshot_log" ]; then
    pass "__gpy_request still falls back to oneshot when the socket is missing"
else
    fail "missing socket: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

# Connection refused is "unreachable", not "late", so it must fall back.
shell_e2e_stub_listener "$tmp/dead.sock" bind-only
out="$(run_request "$tmp/dead.sock")"
if [ "$out" = ONESHOT_OUTPUT ] && [ -s "$oneshot_log" ]; then
    pass "__gpy_request still falls back to oneshot when nothing listens on the socket"
else
    fail "dead socket: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

test_finish "bash slow-reply policy holds on every available transport"
