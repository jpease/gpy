#!/usr/bin/env zsh
# tests/zsh/ipc_partial_response.test.zsh
#
# A truncated (partial-write) IPC response must never reach the caller as if
# it were complete. If any non-empty read counted as a valid response, an
# agent that wrote `{"status":"o` and then stalled or died mid-write would
# have that exact truncated string handed to __gpy_request_character/_duration
# (raw ANSI, echoed straight into the prompt) — corrupting terminal rendering.
#
# The "wedged-mid-write agent" is a stub listener in partial mode: it writes
# a partial (non-newline-terminated) frame, then goes silent. It exits after a
# few seconds so this test has a hard, deterministic upper bound even if the
# client hangs waiting for more bytes.
#
# A second case guards against over-strict framing: a normally-terminated
# complete response must still be returned intact.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a partial-write agent"
fi
if ! (( $+commands[socat] )) && ! (( $+commands[nc] )); then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir=$(mktemp -d)
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# --- Case 1: partial write then stall must not leak the truncated bytes ---

sock1="$test_tmp_dir/gpy-partial-test.sock"
shell_e2e_stub_listener "$sock1" partial --reply '{"status":"o' --lifetime 3

export GPY_AGENT_SOCKET_PATH="$sock1"
result="$(__gpy_send_json '{"op":"ping"}')"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

if [[ -n "$result" ]]; then
    fail "partial write is discarded, not returned: got '$result'"
else
    pass "partial write is discarded, not returned"
fi

# --- Case 2: a complete, newline-terminated response still round-trips ---

sock2="$test_tmp_dir/gpy-complete-test.sock"
shell_e2e_stub_listener "$sock2" reply --reply '{"status":"ok"}'

export GPY_AGENT_SOCKET_PATH="$sock2"
complete_result="$(__gpy_send_json '{"op":"ping"}')"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

if [[ "$complete_result" == '{"status":"ok"}' ]]; then
    pass "complete response still round-trips"
else
    fail "complete response still round-trips: got '$complete_result'"
fi

test_finish
