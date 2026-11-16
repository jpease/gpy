#!/usr/bin/env zsh
# tests/zsh/doorbell_during_precmd.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A doorbell (SIGURG) that arrives while precmd is still rendering (the agent
# finished a refresh the render itself fired) runs TRAPURG with zle inactive,
# so it cannot repaint. It must not be lost: TRAPURG only marks a repaint
# pending (and leaves the agent's flags alone, so no reload lands mid-render),
# and the zle-line-init hook then consumes the flags, re-renders and redraws.
# A ring outside precmd with zle inactive (a command running) still consumes
# the flags at once, as before.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-doorbell-precmd.XXXXXX") || exit 1
SANDBOX=${SANDBOX:A}
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source zsh/gpy.zsh
__gpy_add_newline=0 __gpy_two_line=0 __gpy_cfg_transient_prompt=0
__gpy_shell_flag_base="$SANDBOX/flag"
RELOADS="$SANDBOX/reloads"
: >"$RELOADS"
function __gpy_reload_config() { print -r -- x >>"$RELOADS"; }

# A segment that, mid-render, leaves a reload flag and rings this shell.
function __gpy_segment_ringer() {
    : >"$__gpy_shell_flag_base.reload"
    kill -URG $$
    printf 'RING;'
}
__enabled_segments=(ringer)

true
__gpy_precmd
check "a ring during precmd marks a repaint pending" 1 "$__gpy_repaint_pending"
check "and leaves the reload flag for later" true \
    "$([[ -e $__gpy_shell_flag_base.reload ]] && echo true || echo false)"
check "and reloads nothing mid-render" "" "$(<"$RELOADS")"
check "precmd is over" "" "$__gpy_in_precmd"

# zle-line-init: zle is active there; record the redraw.
ZLE_LOG="$SANDBOX/zle"
: >"$ZLE_LOG"
function zle() { print -r -- "$*" >>"$ZLE_LOG"; return 0; }
function __gpy_segment_ringer() { printf 'AFTER;'; }
__gpy_line_init
check "zle-line-init clears the pending repaint" "" "$__gpy_repaint_pending"
check "consumes the reload flag" false \
    "$([[ -e $__gpy_shell_flag_base.reload ]] && echo true || echo false)"
check "runs the reload once" "x" "$(<"$RELOADS")"
check "re-renders the prompt" true "$([[ $PROMPT == *AFTER\;* ]] && echo true || echo false)"
check "and redraws it" "reset-prompt" "$(<"$ZLE_LOG")"

: >"$ZLE_LOG"
__gpy_line_init
check "without a pending repaint zle-line-init redraws nothing" "" "$(<"$ZLE_LOG")"
unfunction zle

# Outside precmd, with zle inactive, a ring consumes the flags at once.
: >"$RELOADS"
: >"$__gpy_shell_flag_base.reload"
TRAPURG
check "a ring outside precmd consumes the reload flag at once" "x" "$(<"$RELOADS")"
check "and marks no repaint pending" "" "$__gpy_repaint_pending"

# A ring that interrupts a function defined under `emulate sh` (KSH_ARRAYS on,
# as in plugin managers and test harnesses) runs the handler without errors.
emulate sh -c 'gpy_test_sh_loop() { i=0; while [ $i -lt 3 ]; do kill -URG $$; i=$((i+1)); done; echo done; }'
# In this shell, not `$(...)`: `kill -URG $$` rings this shell.
{ gpy_test_sh_loop >/dev/null; } 2>"$SANDBOX/urg.err"
check "a ring inside an emulate-sh function prints no error" "" "$(<"$SANDBOX/urg.err")"

test_finish "a doorbell during precmd is repainted at zle-line-init"
