#!/usr/bin/env zsh
# tests/zsh/ipc_late_agent_negative_cache.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# An agent that accepts connections but never answers (stopped, paused across
# sleep, wedged) costs one reply budget, not one per request: after a send
# the agent answers from memory (ping, register, character, clock, ...) waits
# out its budget, the next sends to that socket return 2 ("late") at once for
# a few seconds, and a segment whose request timed out in its subshell marks
# the shell itself, so the rest of the render skips IPC too. A path-bearing
# request (git, lang, directory) can be slow for its path alone and opens no
# window, as in Bash. A doorbell (the agent answering again) clears the mark.
#
# A silent stub listener plays the wedged agent and logs every request.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

test_require_command python3 "python3 not installed, cannot simulate a wedged agent"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-late-agent.XXXXXX") || exit 1
tmp=${tmp:A}
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/bin"
# `gpy-agent start` at source time must not reach a real agent.
print -r -- '#!/bin/sh
exit 0' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"

requests="$tmp/requests"
sock="$tmp/wedged.sock"

# --- direct sends ------------------------------------------------------------
shell_e2e_stub_listener "$sock" silent --requests "$requests" --lifetime 60
out=$(
    export GPY_AGENT_SOCKET_PATH="$sock" GPY_AGENT_ENABLED=1 GPY_IPC_TIMEOUT_MS=100
    source zsh/core/ipc.zsh
    __gpy_send_json '{"op":"ping","n":1}' "" r; s1=$?
    t1=$EPOCHREALTIME
    __gpy_send_json '{"op":"ping","n":2}' "" r; s2=$?
    t2=$EPOCHREALTIME
    fast=0
    (( t2 - t1 < 0.05 )) && fast=1
    # A doorbell clears the mark (TRAPURG sets it to 0).
    __gpy_agent_late_until=0
    __gpy_send_json '{"op":"ping","n":3}' "" r; s3=$?
    print -r -- "s1=$s1 s2=$s2 fast=$fast s3=$s3"
) 2>/dev/null
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
check "a timed-out send, a gated send, a send after the doorbell" \
    "s1=2 s2=2 fast=1 s3=2" "$out"
check "the gated send never reached the agent" \
    "1 3" "$(grep -o '"n":[0-9]' "$requests" | cut -d: -f2 | tr '\n' ' ' | sed 's/ $//')"

# --- a path-bearing request opens no window ----------------------------------
: >"$requests"
sock="$tmp/wedged-path.sock"
shell_e2e_stub_listener "$sock" silent --requests "$requests" --lifetime 60
out=$(
    export GPY_AGENT_SOCKET_PATH="$sock" GPY_AGENT_ENABLED=1 GPY_IPC_TIMEOUT_MS=100
    source zsh/core/ipc.zsh
    __gpy_send_json '{"op":"git","cwd":"/","n":1}' "" r; s1=$?
    __gpy_send_json '{"op":"ping","n":2}' "" r; s2=$?
    print -r -- "s1=$s1 s2=$s2"
) 2>/dev/null
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
check "a timed-out git request leaves the next request ungated" \
    "1 2" "$(grep -o '"n":[0-9]' "$requests" | cut -d: -f2 | tr '\n' ' ' | sed 's/ $//')"

# --- a render: a segment's timeout marks the shell ---------------------------
: >"$requests"
sock="$tmp/wedged-render.sock"
shell_e2e_stub_listener "$sock" silent --requests "$requests" --lifetime 60
out=$(
    source "$ROOT/tests/lib/supervisor_off.zsh"
    export GPY_AGENT_SOCKET_PATH="$sock" PATH="$tmp/bin:$PATH" GPY_IPC_TIMEOUT_MS=100
    source zsh/gpy.zsh
    export GPY_AGENT_ENABLED=1
    __gpy_is_root=0 __clock_format=1
    __enabled_segments=(clock)
    __gpy_render_prompt 0
    late=0
    (( EPOCHSECONDS < __gpy_agent_late_until )) && late=1
    print -r -- "late=$late"
) 2>/dev/null
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"
check "a segment that timed out in its subshell marks the shell late" "late=1" "$out"
check "the character request after it was skipped" \
    "clock" "$(grep -o '"op":"[a-z_]*"' "$requests" | cut -d'"' -f4 | tr '\n' ' ' | sed 's/ $//')"

test_finish "a late agent is not asked again for a few seconds"
