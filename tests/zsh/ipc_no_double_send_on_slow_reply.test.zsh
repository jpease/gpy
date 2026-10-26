#!/usr/bin/env zsh
# tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh
#
# A connected agent that has not replied must
# get exactly one request, and __gpy_send_json must return status 2 with no
# output when its response budget expires.
#
# Do not schedule a reply a fixed interval after accept(): the client can be
# descheduled between connecting and entering read -t, making even a "late"
# reply available before its read budget begins. Instead a silent stub
# listener holds every accepted socket open without replying until the
# synchronous send returns. Stopping it then makes it accept and record what
# is still queued, including any socat/nc fallback re-send. No sleep decides
# whether the response timed out or how many requests reached the listener.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/ipc.zsh

test_require_command python3 "python3 not installed, cannot simulate a slow-to-reply agent"
if ! zmodload zsh/net/socket 2>/dev/null; then
    test_skip "zsh/net/socket not available, cannot exercise the builtin transport"
fi

test_tmp_dir=$(mktemp -d)
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

sock="$test_tmp_dir/gpy.sock"
connections="$test_tmp_dir/connections"
requests="$test_tmp_dir/requests"
shell_e2e_stub_listener "$sock" silent --connections "$connections" --requests "$requests"
listener_pid=$SHELL_E2E_STUB_PID

# Pin the premise instead of inheriting a caller budget or agent-free mode.
export GPY_AGENT_SOCKET_PATH="$sock" GPY_IPC_TIMEOUT_MS=100 GPY_AGENT_ENABLED=1
result="$(__gpy_send_json '{"op":"ping"}')"
send_rc=$?

# The listener still running proves it held the connection open without
# replying until the client returned; its lifetime running out (and the EOF
# that comes with it) cannot satisfy the timeout check.
if kill -0 "$listener_pid" 2>/dev/null; then
    pass "the listener was still holding the connection when the client returned"
else
    fail "the listener exited before the client returned"
fi
shell_e2e_stop_pids "$listener_pid"

# Primary assertion: the slow reply must
# not have caused a second request over a different connection.
check "exactly one connection was made for one logical request" 1 \
    "$(shell_e2e_count_lines "$connections")"

# Status 2 distinguishes an actual connected response timeout from a missing
# socket/client (status 1); the fixture has not sent a byte or closed a socket.
if (( send_rc == 2 )) && [[ -z "$result" ]]; then
    pass "__gpy_send_json timed out with status 2 and no response"
else
    fail "expected status 2 and no response, got status $send_rc and '$result'"
fi

# Count the request itself too, not just accepted sockets: an empty connection
# or two sends on the same connection must not pass the no-double-send check.
if printf '%s\n' '{"op":"ping"}' | cmp -s - "$requests"; then
    pass "listener received exactly one complete ping request"
else
    fail "listener did not receive exactly one complete ping request"
fi

test_finish
