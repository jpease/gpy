#!/usr/bin/env zsh
# tests/zsh/render_memo_clock_status.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The agent's clock and exit-status pills depend only on the theme, the
# position and prev_bg (plus success for status), so the render loop memoizes
# them like the character: one request, then none until the key changes or
# __gpy_clear_render_memos runs (reload, re-register). The local fallback is
# never memoized, so the agent's pill comes back once it answers again.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-memo-clock.XXXXXX") || exit 1
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source zsh/gpy.zsh
__gpy_add_newline=0 __gpy_two_line=0 __gpy_is_root=0
function __gpy_request_character() { printf 'CHAR'; }

CALLS="$SANDBOX/calls"
calls() { local c; c=$(<"$CALLS"); print -r -- ${#c}; }

# --- clock ---------------------------------------------------------------------
: >"$CALLS"
function __gpy_request_clock() { printf x >>"$CALLS"; printf 'AGENTCLOCK'; }
__clock_format=1
__enabled_segments=(clock)
__gpy_clear_render_memos
__gpy_render_prompt 0
__gpy_render_prompt 0
check "the agent clock is requested once for two renders" 1 "$(calls)"
check "and the memo serves it" true \
    "$([[ $__gpy_render_out == *AGENTCLOCK* ]] && echo true || echo false)"
__gpy_clear_render_memos
__gpy_render_prompt 0
check "clearing the render memos asks again" 2 "$(calls)"

# The local fallback (no reply) is drawn but never memoized.
: >"$CALLS"
function __gpy_request_clock() { printf x >>"$CALLS"; return 1; }
__gpy_clear_render_memos
__gpy_render_prompt 0
__gpy_render_prompt 0
check "without an agent reply the clock is asked on every render" 2 "$(calls)"
check "and the local clock is drawn" true \
    "$([[ $__gpy_render_out == *'%D{'* ]] && echo true || echo false)"

# --- status --------------------------------------------------------------------
: >"$CALLS"
function __gpy_request_exit_status() { printf x >>"$CALLS"; printf 'AGENTSTATUS%s' "$1"; }
__status_format=1
__enabled_segments=(status)
__gpy_clear_render_memos
__gpy_render_prompt 0
__gpy_render_prompt 0
check "the agent status pill is requested once for two successes" 1 "$(calls)"
__gpy_render_prompt 1
check "a failure is a different key" 2 "$(calls)"
check "and draws the failure pill" true \
    "$([[ $__gpy_render_out == *AGENTSTATUS0* ]] && echo true || echo false)"
__gpy_render_prompt 0
check "the memo holds one key, so back to success asks once more" 3 "$(calls)"
check "with the success pill" true \
    "$([[ $__gpy_render_out == *AGENTSTATUS1* ]] && echo true || echo false)"

test_finish "clock and status renders are memoized"
