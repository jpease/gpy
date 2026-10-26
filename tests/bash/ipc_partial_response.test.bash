#!/usr/bin/env bash
# tests/bash/ipc_partial_response.test.bash
#
# A truncated IPC response must never reach the caller as if complete; a
# complete response must round-trip, preserving whitespace and backslashes.
# The listener self-destructs, bounding the test if the client hangs.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a partial-write agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

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

# Fragments end in a space and the clock embeds `\D{..}`; IFS splitting or
# backslash mangling would change the prompt.

ws_want='  {"status":"ok","text":"\\D{%H} "}  '
sock3="$test_tmp_dir/gpy-whitespace-test.sock"
shell_e2e_stub_listener "$sock3" reply --reply "$ws_want"

export GPY_AGENT_SOCKET_PATH="$sock3"
ws_result="$(__gpy_send_json '{"op":"ping"}')"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

if [[ "$ws_result" == "$ws_want" ]]; then
    pass "leading/trailing whitespace and backslashes are preserved"
else
    fail "whitespace/backslashes preserved: got '$ws_result', want '$ws_want'"
fi

test_finish
