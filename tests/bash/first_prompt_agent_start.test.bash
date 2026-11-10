#!/usr/bin/env bash
# tests/bash/first_prompt_agent_start.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The shell holds no protocol or version knowledge: it runs `gpy-agent start`
# exactly once at startup even when the agent already answers (`start` keeps or
# replaces the agent itself), never again while the agent keeps answering, and
# still starts an unreachable agent, capped by the restart attempt limit.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a mock agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-first-start.XXXXXX")"
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$SANDBOX"
}
trap cleanup EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"

calls="$SANDBOX/agent.calls"
cat >"$SANDBOX/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >>'$calls'
exit 0
EOF
chmod +x "$SANDBOX/gpy-agent"
export PATH="$SANDBOX:$PATH"

starts() {
    [[ -f "$calls" ]] || {
        echo 0
        return
    }
    grep -c '^start$' "$calls"
}
# shellcheck disable=SC2329 # invoked through shell_e2e_poll
start_recorded() { (($(starts) >= 1)); }
# shellcheck disable=SC2329 # invoked through shell_e2e_poll
starts_at_least() { (($(starts) >= $1)); }

# Source gpy.bash (whose startup is the first-prompt start) in a subshell, then
# run $1 supervisor checks.
shell_session() {
    (
        # Some Bash versions run the parent's EXIT trap (the sandbox cleanup) here.
        trap - EXIT
        export GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
        # shellcheck source=bash/gpy.bash
        source "$ROOT/bash/gpy.bash" &>/dev/null
        # The prompt-time supervisor checks, all past the rate limit.
        local _
        for _ in $(seq 1 "$1"); do
            __gpy_supervisor_check &>/dev/null
        done
        # The check `disown`s its start; give it a moment to run.
        sleep 0.3
    )
}

# --- (a)+(b): a healthy agent is started once, and not again ---

rm -f "$calls"
sock="$SANDBOX/healthy.sock"
shell_e2e_stub_listener "$sock" reply --lifetime 30 --reply '{"status":"ok"}'
export GPY_AGENT_SOCKET_PATH="$sock"
shell_session 3
shell_e2e_poll 5 start_recorded || fail "a healthy agent did not get a start at shell startup"
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a healthy agent gets exactly one start per shell" 1 "$(starts)"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- (c): an unreachable agent is started, capped by the attempt limit ---

rm -f "$calls"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=2
shell_session 6
# One startup start plus two capped restarts.
shell_e2e_poll 5 starts_at_least 3 || fail "an unreachable agent was not started up to the attempt cap"
sleep 0.5
check "an unreachable agent gets the startup start plus the capped restarts" 3 "$(starts)"

test_finish "agent start once per shell"
