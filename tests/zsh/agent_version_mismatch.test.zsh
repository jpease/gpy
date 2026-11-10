#!/usr/bin/env zsh
# tests/zsh/agent_version_mismatch.test.zsh
#
# An agent left running across an upgrade (same protocol, older version) must
# be replaced: the first __gpy_start_agent runs `gpy-agent start`, which decides
# whether to replace it. A matching agent, an unreadable binary version and
# every later call are left alone.

ROOT="$(cd "$(dirname "${(%):-%x}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh
source zsh/core/supervisor.zsh

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

calls="$test_tmp_dir/agent.calls"
cat >"$test_tmp_dir/gpy-agent" <<EOF
#!/bin/sh
if [ "\$1" = --version ]; then
    [ -n "\$FAKE_BINARY_VERSION" ] && echo "gpy-agent \$FAKE_BINARY_VERSION"
    exit 0
fi
echo "\$*" >>'$calls'
exit 0
EOF
chmod +x "$test_tmp_dir/gpy-agent"
export PATH="$test_tmp_dir:$PATH"
export FAKE_BINARY_VERSION=1.2.3

reply_for() {
    print -r -- '{"AgentStatus":{"version":"'$1'","protocol_version":2,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'
}

# Serve an agent reporting version $1 on a fresh socket.
serve_agent() {
    rm -f "$calls"
    __gpy_agent_version_checked=0
    local sock="$test_tmp_dir/agent-$((++serve_count)).sock"
    shell_e2e_stub_listener "$sock" reply --reply "$(reply_for "$1")"
    export GPY_AGENT_SOCKET_PATH="$sock"
}

starts() {
    [[ -f "$calls" ]] || { print 0; return; }
    grep -c '^start$' "$calls"
}

# --- Case 1: an older agent is flagged, once ---

serve_agent 1.0.0
if __gpy_check_agent_version; then
    fail "an older agent is flagged: expected nonzero return"
else
    pass "an older agent is flagged"
fi
if __gpy_check_agent_version; then
    pass "the version is checked only once per shell"
else
    fail "the version is checked only once per shell: second call returned nonzero"
fi
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 2: a matching agent passes ---

serve_agent 1.2.3
if __gpy_check_agent_version; then
    pass "a matching agent is not flagged"
else
    fail "a matching agent is not flagged: expected zero return"
fi
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 3: an unreadable binary version passes ---

serve_agent 1.0.0
FAKE_BINARY_VERSION='' __gpy_check_agent_version
if (( $? == 0 )); then
    pass "an unreadable binary version is not flagged"
else
    fail "an unreadable binary version is not flagged: expected zero return"
fi
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 4: __gpy_start_agent starts over an older agent ---

serve_agent 1.0.0
__gpy_start_agent
start_recorded() { (( $(starts) >= 1 )); }
shell_e2e_poll 5 start_recorded || fail "an older agent did not make __gpy_start_agent run start"
check "an older same-protocol agent is started over exactly once" 1 "$(starts)"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- Case 5: __gpy_start_agent leaves a matching agent alone ---

serve_agent 1.2.3
__gpy_start_agent
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a matching agent is left alone" 0 "$(starts)"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

test_finish
