#!/usr/bin/env zsh
# tests/zsh/prompt_bang_respected.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY leaves the user's `prompt_bang` alone. With it on, __gpy_render doubles
# each data `!` in the text it puts into PROMPT and __gpy_transient, so a
# directory named `a!b` still draws as `a!b` (with the transient prompt on,
# and off with the clock, where the transient line is the whole prompt), and
# the user's own `RPROMPT='!'` keeps showing the history number. With it off
# the bytes are unchanged. The real binary draws the prompt; no agent.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1
test_require_command gpy

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-bang.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_RUNTIME_DIR="$SANDBOX/run"
config="$XDG_CONFIG_HOME/gpy/config.toml"

# use_config SEGMENTS TRANSIENT: the text theme with these segments.
use_config() {
    printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = [%s]\ntransient_prompt = %s\n\n[agent]\nenabled = false\n' \
        "$1" "$2" >"$config"
}

setopt prompt_bang
source zsh/gpy.zsh

[[ -o prompt_bang ]]
check "prompt_bang is still set after GPY loads" 0 $?
[[ -o prompt_subst && -o prompt_percent ]]
check "prompt_subst and prompt_percent are set" 0 $?

# $1 as zsh draws it, SGR removed.
shown() {
    print -rP -- "$1" | sed -e $'s/\x1b\\[[0-9;]*m//g'
}

dir="$SANDBOX/a!b"
mkdir -p "$dir"
cd "$dir" || exit 1
RPROMPT='!'

# --- transient prompt on, no clock ---------------------------------------------
use_config '"directory"' true
__gpy_precmd
[[ "$(shown "$PROMPT")" == *"a!b"* ]]
check "transient on: the directory a!b draws as a!b" 0 $?
[[ "$(shown "$PROMPT")" != *"a!!b"* ]]
check "transient on: no doubled bang is drawn" 0 $?

# --- transient prompt on, with the clock ---------------------------------------
use_config '"clock", "directory"' true
__gpy_precmd
[[ "$(shown "$PROMPT")" == *"a!b"* ]]
check "transient on, clock: the prompt shows a!b" 0 $?
t=$(shown "$__gpy_transient")
[[ -n "$t" && "$t" != *"!!"* ]]
check "transient on, clock: the transient prompt draws without a doubled bang" 0 $?

# --- transient prompt off, with the clock (the whole prompt) --------------------
use_config '"clock", "directory"' false
__gpy_precmd
[[ "$__gpy_transient" == "$PROMPT" ]]
check "transient off, clock: the transient prompt is the prompt" 0 $?
[[ "$(shown "$__gpy_transient")" == *"a!b"* ]]
check "transient off, clock: the transient prompt shows a!b" 0 $?
[[ "$PROMPT" != *"!!!!"* ]]
check "transient off, clock: no quadrupled bang" 0 $?

# --- the user's own prompt strings ---------------------------------------------
history_number=$(print -rP -- "$RPROMPT")
[[ "$history_number" != '!' && -n "$history_number" ]]
check "RPROMPT='!' still shows the history number" 0 $?

# --- prompt_bang off: bytes unchanged --------------------------------------------
unsetopt prompt_bang
use_config '"directory"' true
__gpy_precmd
[[ "$PROMPT" == *"a!b"* && "$PROMPT" != *"a!!b"* ]]
check "prompt_bang off: the directory text is not doubled" 0 $?

test_finish "prompt_bang respected"
