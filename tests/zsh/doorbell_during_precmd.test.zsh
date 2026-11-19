#!/usr/bin/env zsh
# tests/zsh/doorbell_during_precmd.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A doorbell (SIGURG) that arrives while precmd is still rendering (the agent
# finished a refresh the render itself fired) runs TRAPURG with zle inactive,
# so it cannot repaint. It must not be lost: TRAPURG only marks a repaint
# pending (no second `gpy prompt` mid-render), and the zle-line-init hook then
# re-renders and redraws. A ring outside precmd with zle inactive (a command
# running) renders nothing and marks nothing.
#
# A stub gpy stands in for the binary: it counts its runs and, on the first,
# rings the shell mid-render.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-doorbell-precmd.XXXXXX") || exit 1
SANDBOX=${SANDBOX:A}
trap 'rm -rf "$SANDBOX"' EXIT
RUNS="$SANDBOX/runs"
: >"$RUNS"
mkdir -p "$SANDBOX/bin"
cat >"$SANDBOX/bin/gpy" <<EOF
#!/bin/sh
echo x >>'$RUNS'
if [ "\$(wc -l <'$RUNS')" -eq 1 ]; then
    kill -URG "\$GPY_TEST_SHELL_PID"
    printf 'RING;\037'
else
    printf 'AFTER;\037'
fi
EOF
chmod +x "$SANDBOX/bin/gpy"
path=("$SANDBOX/bin" $path)
export GPY_TEST_SHELL_PID=$$

source zsh/gpy.zsh

runs() { wc -l <"$RUNS" | tr -d ' '; }

true
__gpy_precmd
check "a ring during precmd marks a repaint pending" 1 "$__gpy_repaint_pending"
check "and renders nothing more mid-render" 1 "$(runs)"
check "the prompt is the one precmd drew" "RING;" "$PROMPT"
check "precmd is over" "" "$__gpy_in_precmd"

# zle-line-init: zle is active there; record the redraw.
ZLE_LOG="$SANDBOX/zle"
: >"$ZLE_LOG"
function zle() { print -r -- "$*" >>"$ZLE_LOG"; return 0; }
__gpy_line_init
check "zle-line-init clears the pending repaint" "" "$__gpy_repaint_pending"
check "re-renders the prompt once" 2 "$(runs)"
check "with the new prompt" "AFTER;" "$PROMPT"
check "and redraws it" "reset-prompt" "$(<"$ZLE_LOG")"

: >"$ZLE_LOG"
__gpy_line_init
check "without a pending repaint zle-line-init redraws nothing" "" "$(<"$ZLE_LOG")"
check "and renders nothing" 2 "$(runs)"
unfunction zle

# Outside precmd, with zle inactive (a command running), a ring renders
# nothing and marks nothing: the next precmd draws the prompt anyway.
TRAPURG
check "a ring outside precmd renders nothing" 2 "$(runs)"
check "and marks no repaint pending" "" "$__gpy_repaint_pending"

# A ring that interrupts a function defined under `emulate sh` (KSH_ARRAYS on,
# as in plugin managers and test harnesses) runs the handler without errors.
emulate sh -c 'gpy_test_sh_loop() { i=0; while [ $i -lt 3 ]; do kill -URG $$; i=$((i+1)); done; echo done; }'
# In this shell, not `$(...)`: `kill -URG $$` rings this shell.
{ gpy_test_sh_loop >/dev/null; } 2>"$SANDBOX/urg.err"
check "a ring inside an emulate-sh function prints no error" "" "$(<"$SANDBOX/urg.err")"

test_finish "a doorbell during precmd is repainted at zle-line-init"
