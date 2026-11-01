#!/usr/bin/env zsh
# tests/zsh/prompt_bang_respected.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY leaves the user's `prompt_bang` alone. With it on, __gpy_update_prompts
# doubles each data `!` in the text it puts into PROMPT and
# __gpy_transient_prompt, so a directory named `a!b` still draws as `a!b`
# (with transient prompt on, and off with the clock, where the transient
# prompt is copied from PROMPT), and the user's own `RPROMPT='!'` keeps
# showing the history number.
# The agent is disabled; the directory stub escapes like the agent's
# `zsh-prompt` format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-bang.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

setopt prompt_bang
source zsh/gpy.zsh

[[ -o prompt_bang ]]
check "prompt_bang is still set after GPY loads" 0 $?
[[ -o prompt_subst && -o prompt_percent ]]
check "prompt_subst and prompt_percent are set" 0 $?

function __gpy_segment_directory() {
    local esc
    __gpy_prompt_escape "$PWD" esc
    printf '%s' "$esc"
}

# $1 as zsh draws it, SGR removed.
shown() {
    print -rP -- "$1" | sed -e $'s/\x1b\\[[0-9;]*m//g'
}

__segment_delim_first="<" __segment_delim_start="[" __segment_delim_end="]" __segment_delim_last=")"
__color_clock_bg="blue" __color_clock_fg="yellow"
__color_directory_bg="black" __color_directory_fg="white"
__time_format=12 __clock_show_leading_zero=0 __clock_show_seconds=0
__prompt_time_pad=" "
__icon_prompt="❯" __prompt_color="cyan"
GPY_SHOW_STATUS=0
__gpy_two_line=0 __gpy_add_newline=0
__gpy_is_root=0

dir="$SANDBOX/a!b"
mkdir -p "$dir"
cd "$dir" || exit 1
RPROMPT='!'

# --- transient prompt on, no clock ---------------------------------------------
__gpy_cfg_transient_prompt=1
__enabled_segments=(directory)
__gpy_char_cache_key="" __gpy_char_cache_val="" __gpy_dir_cache_key="" __gpy_dir_cache_val=""
__gpy_update_prompts 0
[[ "$(shown "$PROMPT")" == *"a!b"* ]]
check "transient on: the directory a!b draws as a!b" 0 $?
[[ "$(shown "$PROMPT")" != *"a!!b"* ]]
check "transient on: no doubled bang is drawn" 0 $?
[[ "$__gpy_dir_cache_val" == *"a!b"* && "$__gpy_dir_cache_val" != *"a!!b"* ]]
check "the directory memo keeps the raw text" 0 $?

# --- transient prompt on, with the clock ---------------------------------------
__enabled_segments=(clock directory)
__gpy_update_prompts 0
[[ "$(shown "$PROMPT")" == *"a!b"* ]]
check "transient on, clock: the prompt shows a!b" 0 $?
t=$(shown "$__gpy_transient_prompt")
[[ "$t" == *"❯"* && "$t" != *"!!"* ]]
check "transient on, clock: the transient prompt draws without a doubled bang" 0 $?

# --- transient prompt off, with the clock (copied from PROMPT) ------------------
__gpy_cfg_transient_prompt=0
__gpy_update_prompts 0
[[ "$__gpy_transient_prompt" == "$PROMPT" ]]
check "transient off, clock: the transient prompt is the prompt, not doubled again" 0 $?
[[ "$(shown "$__gpy_transient_prompt")" == *"a!b"* ]]
check "transient off, clock: the transient prompt shows a!b" 0 $?
[[ "$PROMPT" != *"!!!!"* ]]
check "transient off, clock: no quadrupled bang" 0 $?

# --- the user's own prompt strings ---------------------------------------------
history_number=$(print -rP -- "$RPROMPT")
[[ "$history_number" != '!' && -n "$history_number" ]]
check "RPROMPT='!' still shows the history number" 0 $?

# --- prompt_bang off: bytes unchanged --------------------------------------------
unsetopt prompt_bang
__gpy_cfg_transient_prompt=1
__enabled_segments=(directory)
__gpy_update_prompts 0
[[ "$PROMPT" == *"a!b"* && "$PROMPT" != *"a!!b"* ]]
check "prompt_bang off: the directory text is not doubled" 0 $?

test_finish "prompt_bang respected"
