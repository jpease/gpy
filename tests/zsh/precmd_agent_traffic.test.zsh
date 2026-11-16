#!/usr/bin/env zsh
# tests/zsh/precmd_agent_traffic.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# What one prompt sends to an agent that refuses to register:
#   - one `register` per prompt, after the render (never a second, blocking
#     one before it), as in Bash;
#   - the supervisor's health probe is an in-shell `ping` within the IPC
#     budget, never a `gpy-agent status` fork.
# A stub listener answers every request with an error and logs it; a
# recording `gpy-agent` stub logs any CLI call.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

test_require_command python3 "python3 not installed, cannot simulate an agent"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-precmd-traffic.XXXXXX") || exit 1
tmp=${tmp:A}
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/bin"
cli_log="$tmp/cli.log"
print -r -- "#!/bin/sh
echo \"\$*\" >>'$cli_log'
exit 0" >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"
: >"$cli_log"

requests="$tmp/requests"
sock="$tmp/agent.sock"
shell_e2e_stub_listener "$sock" reply --reply '{"error":"refused"}' --requests "$requests" --lifetime 60

(
    source "$ROOT/tests/lib/supervisor_off.zsh"
    export GPY_AGENT_SOCKET_PATH="$sock" PATH="$tmp/bin:$PATH"
    source zsh/gpy.zsh
    export GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1
    export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
    __enabled_segments=()
    __gpy_is_root=1
    : >"$cli_log"
    true
    __gpy_precmd
) >/dev/null 2>&1
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

ops=$(grep -o '"op":"[a-z_]*"' "$requests" | cut -d'"' -f4)
check "one register for one unregistered prompt" 1 "$(print -r -- "$ops" | grep -c '^register$')"
check "the supervisor probes with an IPC ping" 1 "$(print -r -- "$ops" | grep -c '^ping$')"
check "and never forks gpy-agent status" 0 "$(grep -c '^status' "$cli_log")"

test_finish "one prompt's agent traffic"
