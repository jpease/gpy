#!/usr/bin/env zsh
# tests/zsh/first_prompt_agent_start.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A shell's first prompt runs `gpy-agent start` exactly once even when the
# agent already answers (`start` keeps or replaces the agent itself), never
# again while the agent keeps answering, and still starts an unreachable
# agent, capped by the restart attempt limit. `gpy prompt zsh` does this; the
# test drives it through zsh/gpy.zsh's precmd.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1
source "$ROOT/tests/lib/supervisor_off.zsh"
test_require_command gpy
test_require_command python3

test_tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/gpy-first-start.XXXXXX") || exit 1
test_tmp_dir=${test_tmp_dir:A}
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# The binary starts the gpy-agent beside itself: a copy of gpy next to a
# recording stub.
calls="$test_tmp_dir/agent.calls"
mkdir -p "$test_tmp_dir/bin"
cp "${commands[gpy]}" "$test_tmp_dir/bin/gpy"
cat >"$test_tmp_dir/bin/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >>'$calls'
exit 0
EOF
chmod +x "$test_tmp_dir/bin/gpy-agent"

starts() {
    [[ -f "$calls" ]] || { print 0; return; }
    grep -c '^start$' "$calls"
}
starts_at_least() { (( $(starts) >= $1 )); }

# shell_session N: a fresh zsh (fresh PID, so a first prompt) draws N
# prompts, every one past the supervisor's rate limit.
shell_session() {
    rm -rf "$test_tmp_dir/run"
    mkdir -p "$test_tmp_dir/run"
    (
        path=("$test_tmp_dir/bin" $path)
        export XDG_RUNTIME_DIR="$test_tmp_dir/run"
        export GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
        zsh -f -c 'source "$1/zsh/gpy.zsh" &>/dev/null
            for i in {1..$2}; do __gpy_precmd; done' zsh "$ROOT" "$1" &>/dev/null
    )
}

# --- (a)+(b): a healthy agent is started once, and not again ---

rm -f "$calls"
sock="$test_tmp_dir/healthy.sock"
shell_e2e_stub_listener "$sock" reply --lifetime 30 --reply '{"status":"ok"}'
export GPY_AGENT_SOCKET_PATH="$sock"
shell_session 3
shell_e2e_poll 5 starts_at_least 1 || fail "a healthy agent did not get a start on the first prompt"
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a healthy agent gets exactly one start per shell" 1 "$(starts)"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# --- (c): an unreachable agent is started, capped by the attempt limit ---

rm -f "$calls"
export GPY_AGENT_SOCKET_PATH="$test_tmp_dir/missing.sock"
export GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=2
shell_session 6
# One startup start plus two capped restarts.
shell_e2e_poll 5 starts_at_least 3 || fail "an unreachable agent was not started up to the attempt cap"
sleep 0.5
check "an unreachable agent gets the startup start plus the capped restarts" 3 "$(starts)"

test_finish "agent start once per shell"
