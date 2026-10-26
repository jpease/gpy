#!/usr/bin/env zsh
# tests/zsh/protocol_version_mismatch.test.zsh
#
# An agent that responds to IPC but speaks a
# different wire protocol (e.g. an old daemon left running across an
# upgrade) must be detected shell-side, not silently treated as usable.
#
# `gpy-agent status` exits 0 for any responsive agent regardless of protocol
# version, so it cannot be __gpy_start_agent's sole "already running" gate.
# This exercises __gpy_check_protocol_version directly.

ROOT="$(cd "$(dirname "${(%):-%x}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a mock agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# --- Case 1: mismatched protocol_version must be detected ---

sock1="$test_tmp_dir/gpy-protocol-mismatch.sock"
shell_e2e_stub_listener "$sock1" reply \
    --reply '{"AgentStatus":{"version":"9.9.9","protocol_version":99,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'

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

test_finish
