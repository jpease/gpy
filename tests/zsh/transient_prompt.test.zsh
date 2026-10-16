#!/usr/bin/env zsh
# tests/zsh/transient_prompt.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_update_prompts sets PROMPT and __gpy_transient_prompt, the prompt
# zle-line-finish redraws when a line is submitted:
#   - ui.transient_prompt on: the clock's live `%D{...}` time in its text
#     color with no pill, then the prompt symbol alone (no status indicator,
#     no other segment); root keeps `!❯!` in its color;
#   - a symbol without a leading space still gets one after the time;
#   - on with no clock: the symbol alone;
#   - off with the clock: the full prompt, re-expanded at the redraw so the
#     time is the one Enter was pressed;
#   - off with no clock: empty, so nothing is redrawn.
# The agent is unreachable, so the symbol is the shell-drawn fallback.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-transient.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source zsh/gpy.zsh

FAILED=0
check() {
    local label=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [${(q+)expected}], got [${(q+)actual}])"
        FAILED=1
    fi
}
# $1 with every `%{...%}` region (the non-printing SGR) removed: what is drawn.
printed() { printf '%s' "$1" | sed -e 's/%{[^%]*%}//g'; }

__segment_delim_first="<" __segment_delim_start="[" __segment_delim_end="]" __segment_delim_last=")"
__color_clock_bg="blue" __color_clock_fg="yellow"
__color_directory_bg="black" __color_directory_fg="white"
__time_format=12 __clock_show_leading_zero=0 __clock_show_seconds=0
__prompt_time_pad=" "
__icon_prompt="❯" __prompt_color="cyan"
__icon_root_prompt="!❯!" __root_prompt_color="red"
__icon_status_ok="OK" __icon_status_fail="NO"
GPY_SHOW_STATUS=1
__gpy_two_line=1 __gpy_add_newline=0
__gpy_is_root=0
__gpy_char_cache_key="" __gpy_char_cache_val=""

clock_spec='%D{%-I:%M %p}'

# --- on, with the clock -----------------------------------------------------
GPY_UI_TRANSIENT_PROMPT=1
__enabled_segments=(clock directory)
__gpy_update_prompts 1
check "on: time, then the symbol only (no status, no segments)" \
    " ${clock_spec} ❯ " "$(printed "$__gpy_transient_prompt")"
[[ "$__gpy_transient_prompt" == "%{"$'\e'"[33;49m%} ${clock_spec}"* ]]
check "on: the time keeps the clock's text color, without its background" 0 $?
[[ "$__gpy_transient_prompt" == *"%{"$'\e'"[36;49m%}❯ "* ]]
check "on: the symbol keeps the prompt color" 0 $?
[[ "$PROMPT" != *$'\x1e'* && "$PROMPT" == *NO* && "$PROMPT" == *$'\n'* ]]
check "the full prompt is unchanged (status indicator, two lines)" 0 $?

__gpy_add_newline=1
__gpy_update_prompts 0
check "on: the blank line before the prompt is kept" \
    $'\n'" ${clock_spec} ❯ " "$(printed "$__gpy_transient_prompt")"
__gpy_add_newline=0

__gpy_is_root=1
__gpy_update_prompts 0
check "root: the root symbol" " ${clock_spec} !❯! " "$(printed "$__gpy_transient_prompt")"
[[ "$__gpy_transient_prompt" == *"%{"$'\e'"[31;49m%}!❯! "* ]]
check "root: the symbol keeps the root color" 0 $?
__gpy_is_root=0

# --- on, without the clock --------------------------------------------------
__enabled_segments=(directory)
__gpy_update_prompts 0
check "on, no clock: the symbol alone" "❯ " "$(printed "$__gpy_transient_prompt")"

# --- off --------------------------------------------------------------------
GPY_UI_TRANSIENT_PROMPT=0
__enabled_segments=(clock directory)
__gpy_update_prompts 0
check "off, with the clock: the full prompt is redrawn" "$PROMPT" "$__gpy_transient_prompt"

__enabled_segments=(directory)
__gpy_update_prompts 0
check "off, no clock: nothing to redraw" "" "$__gpy_transient_prompt"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
