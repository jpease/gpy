#!/usr/bin/env bash
# tests/bash/protocol_version_mismatch.test.bash
#
# An agent that responds to IPC but speaks a different wire protocol (e.g. an
# old daemon left running across an upgrade) must be detected shell-side,
# not silently treated as usable.
#
# This exercises __gpy_check_protocol_version directly, and confirms it gates
# __gpy_supervisor_is_running (not just a ping) so a responsive-but-mismatched
# agent is treated as not running.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source bash/core/ipc.bash
source bash/core/supervisor.bash

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a mock agent"
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

mismatched_status='{"AgentStatus":{"version":"9.9.9","protocol_version":99,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'

# --- Case 1: mismatched protocol_version must be detected ---

sock1="$test_tmp_dir/gpy-protocol-mismatch.sock"
shell_e2e_stub_listener "$sock1" reply --reply "$mismatched_status"

export GPY_AGENT_SOCKET_PATH="$sock1"

if __gpy_check_protocol_version; then
    fail "mismatched protocol_version is detected: expected nonzero return"
else
    pass "mismatched protocol_version is detected"
fi

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 2: matching protocol_version must not be flagged ---

sock2="$test_tmp_dir/gpy-protocol-match.sock"
shell_e2e_stub_listener "$sock2" reply \
    --reply '{"AgentStatus":{"version":"0.1.0","protocol_version":2,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'

export GPY_AGENT_SOCKET_PATH="$sock2"

if __gpy_check_protocol_version; then
    pass "matching protocol_version is not flagged"
else
    fail "matching protocol_version is not flagged: expected zero return"
fi

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 3: __gpy_supervisor_is_running treats a mismatch as not running ---

# The ping is answered like a healthy agent's, so only the status query can
# tell it apart.
sock3="$test_tmp_dir/gpy-protocol-supervisor.sock"
shell_e2e_stub_listener "$sock3" reply --reply "$mismatched_status" \
    --route '"op":"ping"' '{"status":"ok"}' --requests "$test_tmp_dir/requests"

export GPY_AGENT_SOCKET_PATH="$sock3"

if __gpy_supervisor_is_running; then
    fail "supervisor treats mismatched-protocol agent as running: expected nonzero return"
else
    pass "supervisor treats mismatched-protocol agent as not running"
fi

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
check "supervisor queried the status after a healthy ping" 1 \
    "$(grep -c '"op":"status"' "$test_tmp_dir/requests")"

test_finish
