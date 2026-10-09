#!/usr/bin/env zsh
# tests/zsh/local_segment_renderer.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #844: the segments Zsh draws itself (status pill, local clock, hostname and
# username fallback) and the root prompt go through the shared renderer
# (zsh/core/renderer.zsh), the twin of fish's gpy_section_standalone. The
# byte-for-byte comparison with Fish lives in
# tests/bash/shell_contract.test.bash; this file pins the Zsh-side
# contracts that comparison cannot see (the twin of
# tests/bash/local_segment_renderer.test.bash):
#   - the render loop dispatches `status` like every other segment (is_first /
#     is_last) and the next segment's chevron takes its prev_bg from the pill
#     that was drawn (ok or fail background);
#   - root draws `__icon_root_prompt` in `__root_prompt_color` and never asks
#     the agent for a character; the fallback draws the exit-status indicator
#     unless GPY_SHOW_STATUS=0, as fish_prompt does;
#   - data the renderer prints (hostname, username, icons) shows literally,
#     whatever `$(...)`, backticks, backslashes or `%` it holds (#677);
#   - the color table maps names, bright names, `#RRGGBB` and 0-255.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-local-renderer.XXXXXX")
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
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}
contains() { [[ $1 == *"$2"* ]] && echo true || echo false; }

# Theme state the renderer reads: ASCII delimiters, named colors.
__segment_delim_first="<" __segment_delim_start="[" __segment_delim_end="]" __segment_delim_last=")"
__prompt_open_color="white" __prompt_open_bg="transparent"
__prompt_close_color="white" __prompt_close_bg="transparent"
__segment_delimiter_color="white" __segment_delimiter_bg="transparent"
__color_status_ok_bg="green" __color_status_ok_fg="black"
__color_status_fail_bg="magenta" __color_status_fail_fg="white"
__icon_status_ok="OK" __icon_status_fail="NO"
__gpy_two_line=0 __gpy_add_newline=0
__gpy_char_cache_key="" __gpy_char_cache_val=""

# What zsh draws for prompt source $1 (the prompt options the integration pins).
shown() { print -rnP -- "$1" | sed -e $'s/\033\\[[0-9;]*m//g'; }
# $1 with every `%{...%}` region (the non-printing SGR) removed: what is drawn.
printed() { printf '%s' "$1" | sed -e 's/%{[^%]*%}//g'; }
# $1 with only the `%{` / `%}` markers removed: SGR bytes beside their text.
flat() { printf '%s' "$1" | sed -e 's/%{//g' -e 's/%}//g'; }

# --- the color table ---------------------------------------------------------
param() { local out; __gpy_sgr_param "$1" "$2" out; printf '%s' "$out"; }
check "fg named color" "32" "$(param fg green)"
check "bg named color" "44" "$(param bg blue)"
check "fg bright name (brred)" "91" "$(param fg brred)"
check "bg bright name (bright_black)" "100" "$(param bg bright_black)"
check "gray is bright black" "90" "$(param fg gray)"
check "fg hex" "38;2;255;136;0" "$(param fg '#ff8800')"
check "bg hex" "48;2;0;170;85" "$(param bg '#00aa55')"
check "fg 0-255 index" "38;5;200" "$(param fg 200)"
check "bg 0-255 index, zero-padded" "48;5;8" "$(param bg 08)"
check "index above 255 is the default" "49" "$(param bg 300)"
check "transparent is the default fg" "39" "$(param fg transparent)"
check "transparent is the default bg" "49" "$(param bg transparent)"
check "empty is the default" "39" "$(param fg '')"
check "an unknown name is the default, not a wrong color" "49" "$(param bg chartreuse-ish)"

# --- the status pill, through the real dispatch loop -------------------------
# A probe segment after `status` reports the prev_bg the loop handed it.
function __gpy_segment_probe() { printf 'probe:%s;' "$2"; }
__enabled_segments=(status probe)
rendered=$(__gpy_render_prompt 0)
check "after an ok status pill the next segment's prev_bg is the ok background" true \
    "$(contains "$rendered" 'probe:green;')"
rendered=$(__gpy_render_prompt 7)
check "after a failed status pill the next segment's prev_bg is the fail background" true \
    "$(contains "$rendered" 'probe:magenta;')"
check "the fail pill carries the fail icon, not the ok one" true \
    "$([[ $rendered == *NO* && $rendered != *OK* ]] && echo true || echo false)"

# status alone is both the first and the last segment: first cap, last cap
__enabled_segments=(status)
rendered=$(__gpy_render_prompt 0)
check "a lone status pill opens with the first-position cap and closes with the last" true \
    "$([[ $rendered == *"<"*"OK"*")"* ]] && echo true || echo false)"
p=$(printed "$rendered")
check "a lone status pill has no gap-and-middle-cap" true \
    "$([[ $p != *"["* && $p != *"]"* ]] && echo true || echo false)"

# --- the root prompt ---------------------------------------------------------
function __gpy_request_character() { echo x >>"$SANDBOX/char-requests"; printf 'AGENTCHAR'; }
: >"$SANDBOX/char-requests"
__enabled_segments=(probe)
__icon_root_prompt="#!" __root_prompt_color="yellow"
__icon_prompt=">" __prompt_color="cyan"
GPY_SHOW_STATUS=0

__gpy_is_root=0
rendered=$(__gpy_render_prompt 0)
check "non-root: the agent's character is the prompt symbol" true \
    "$([[ $rendered == *AGENTCHAR ]] && echo true || echo false)"

__gpy_is_root=1
: >"$SANDBOX/char-requests"
__gpy_char_cache_key="" __gpy_char_cache_val=""
rendered=$(__gpy_render_prompt 0)
check "root: the agent is never asked for a character" 0 "$(wc -l <"$SANDBOX/char-requests" | tr -d ' ')"
check "root: the symbol is __icon_root_prompt plus a space, outside the escapes" "#! " \
    "$(printed "$rendered" | sed 's/.*probe:[^;]*;//')"
check "root: drawn in __root_prompt_color (yellow, 33)" true \
    "$([[ $(flat "$rendered") == *$'\033[33;49m'"#! "* ]] && echo true || echo false)"
check "root: no agent character in the prompt" true \
    "$([[ $rendered != *AGENTCHAR* ]] && echo true || echo false)"

# GPY_SHOW_STATUS=1 puts fish's exit-status indicator before the root symbol
GPY_SHOW_STATUS=1
rendered=$(__gpy_render_prompt 0)
check "root + GPY_SHOW_STATUS=1: ok indicator, then the root symbol" "OK #! " \
    "$(printed "$rendered" | sed 's/.*probe:[^;]*;//')"
rendered=$(__gpy_render_prompt 2)
check "root + failed command: fail indicator in red, then the root symbol" true \
    "$([[ $(flat "$rendered") == *$'\033[31;49m'"NO "* && $(printed "$rendered") == *"NO #! " ]] && echo true || echo false)"

# The agent returning nothing falls back to the same local symbol (non-root)
__gpy_is_root=0
function __gpy_request_character() { return 0; }
__gpy_char_cache_key="" __gpy_char_cache_val=""
GPY_SHOW_STATUS=0
rendered=$(__gpy_render_prompt 0)
check "agent silent: the local prompt icon in __prompt_color (cyan, 36)" true \
    "$([[ $(flat "$rendered") == *$'\033[36;49m'">"* && $(printed "$rendered") == *"> " ]] && echo true || echo false)"
unfunction __gpy_request_character
source zsh/core/ipc.zsh

# --- data prints literally ---------------------------------------------------
unset __username_format __hostname_format
__color_username_bg="red" __color_username_fg="white" __icon_username=""
for hostile in '$(echo GPY_EXPANDED)' 'tick`echo GPY_TICK`' 'a\u\hb' '100%_done' 'x!y' '\\\\' '${HOME}' '%F{red}x%f'; do
    USER=$hostile
    check "username '$hostile' shows literally" true \
        "$(contains "$(shown "$(__gpy_segment_username true "" true)")" "<$hostile)")"
    HOST=$hostile __hostname_trim_at="" __icon_hostname="" __gpy_is_ssh=1
    check "hostname '$hostile' shows literally" true \
        "$(contains "$(shown "$(__gpy_segment_hostname true "" true)")" "<$hostile)")"
done
__icon_username='$(echo ICON)`%'
USER="u"
check "an icon with shell metacharacters shows literally" true \
    "$(contains "$(shown "$(__gpy_segment_username true "" true)")" '<$(echo ICON)`% u)')"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
