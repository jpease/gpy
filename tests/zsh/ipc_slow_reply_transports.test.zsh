#!/usr/bin/env zsh
# tests/zsh/ipc_slow_reply_transports.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The socat and nc transports follow the same slow-reply policy as zsocket
# (tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh covers zsocket).
#
# An agent that accepted the request but answers late is working on it. The
# shell must (a) put the request on the wire exactly once, never resend over a
# second connection, and (b) report "connected, no reply" (status 2, empty
# output) so no caller recomputes the segment with a blocking
# `gpy-agent oneshot` fork. Only an agent that cannot be reached at all
# (status 1) may fall back to oneshot.
#
# A stub listener plays the slow agent and records one line per accepted
# connection. `zmodload` is shadowed so zsocket is unavailable, and each
# transport that exists on this machine runs on its own PATH: socat, nc without
# `timeout` (nc's own whole-second -w bound), nc wrapped in `timeout` if present.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

test_require_command python3 "python3 not installed, cannot simulate a slow-to-reply agent"

source zsh/core/constants.zsh
source zsh/core/ipc.zsh

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-slow-reply.XXXXXX")
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT

# Hide the builtin transport so the external clients are the ones exercised.
zmodload() { [[ "$1" == "zsh/net/socket" ]] && return 1; builtin zmodload "$@"; }

real_path=$PATH
# make_shim NAME TOOL...: a PATH holding only the named tools.
make_shim() {
    local dir="$tmp/shim-$1" tool src
    shift
    mkdir -p "$dir"
    for tool in "$@"; do
        src=$(command -v "$tool") || return 1
        ln -sf "$src" "$dir/$tool"
    done
    print -rn -- "$dir"
}

# start_listener DELAY: replies `{"status":"ok"}` after DELAY seconds,
# counting accepted connections in $tmp/connections.
start_listener() {
    rm -f "$tmp/slow.sock" "$tmp/connections"
    shell_e2e_stub_listener "$tmp/slow.sock" reply --reply '{"status":"ok"}' \
        --delay "$1" --lifetime 6 --connections "$tmp/connections"
}

# check_transport LABEL PATH DELAY: one slow request over the given PATH.
check_transport() {
    local label=$1 use_path=$2 delay=$3 out send_rc connections
    start_listener "$delay"
    export GPY_AGENT_SOCKET_PATH="$tmp/slow.sock" GPY_IPC_TIMEOUT_MS=150
    PATH=$use_path
    rehash
    out=$(__gpy_send_json '{"op":"ping"}')
    send_rc=$?
    PATH=$real_path
    rehash
    # Window for a fallback resend to connect: a second request has no event
    # to poll for, so wait it out before the count is read.
    sleep 0.3
    shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
    connections=$(shell_e2e_count_lines "$tmp/connections")

    if (( connections == 1 )); then
        pass "$label: one logical request is one connection"
    else
        fail "$label: expected exactly one connection, got $connections"
    fi
    if (( send_rc == 2 )) && [[ -z "$out" ]]; then
        pass "$label: a connected-but-late agent is status 2 with no output"
    else
        fail "$label: status $send_rc, output '$out' (want status 2, empty)"
    fi
}

ran=0
if (( $+commands[socat] )); then
    ran=$((ran + 1))
    check_transport "socat" "$(make_shim socat socat)" 0.5
fi
if (( $+commands[nc] )); then
    nc_shim=$(make_shim nc nc)
    PATH=$nc_shim
    rehash
    __gpy_nc_supports_unix=""
    __gpy_nc_probe_capabilities
    PATH=$real_path
    rehash
    if [[ "$__gpy_nc_supports_unix" == 1 ]]; then
        ran=$((ran + 1))
        # nc's -w is whole seconds, so the reply has to be later than 1s.
        check_transport "nc (no timeout)" "$nc_shim" 1.6
        if (( $+commands[timeout] )); then
            ran=$((ran + 1))
            check_transport "nc (timeout wrapper)" "$(make_shim nct nc timeout)" 0.5
        fi
    fi
fi
(( ran > 0 )) || test_skip "neither socat nor an nc with -U is installed"

# The caller-visible half: __gpy_request never forks oneshot for a late agent,
# and still does for one that cannot be reached. A stub gpy-agent logs calls.
mkdir -p "$tmp/bin" "$tmp/cache"
oneshot_log="$tmp/oneshot.log"
print -r -- '#!/bin/sh
echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"
echo ONESHOT_OUTPUT' > "$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"
request_path="$tmp/bin:$real_path"

# run_request SOCKET: prints the segment text.
run_request() {
    : >"$oneshot_log"
    (
        export GPY_AGENT_SOCKET_PATH="$1" GPY_IPC_TIMEOUT_MS=150 \
            GPY_TEST_ONESHOT_LOG="$oneshot_log" XDG_CACHE_HOME="$tmp/cache" PATH="$request_path"
        rehash
        __gpy_request git "$tmp" ansi true
    )
}

start_listener 0.6
out=$(run_request "$tmp/slow.sock")
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
if [[ -z "$out" && ! -s "$oneshot_log" ]]; then
    pass "__gpy_request omits the segment for a late agent and forks no oneshot"
else
    fail "late agent: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

out=$(run_request "$tmp/missing.sock")
if [[ "$out" == ONESHOT_OUTPUT && -s "$oneshot_log" ]]; then
    pass "__gpy_request still falls back to oneshot when the socket is missing"
else
    fail "missing socket: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

# A socket file nobody listens on (a crashed agent): connection refused is
# "unreachable", not "late", so it must also fall back.
shell_e2e_stub_listener "$tmp/dead.sock" bind-only
out=$(run_request "$tmp/dead.sock")
if [[ "$out" == ONESHOT_OUTPUT && -s "$oneshot_log" ]]; then
    pass "__gpy_request still falls back to oneshot when nothing listens on the socket"
else
    fail "dead socket: output '$out', oneshot log '$(cat "$oneshot_log")'"
fi

test_finish "zsh slow-reply policy holds on every available transport"
