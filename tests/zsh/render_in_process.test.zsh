#!/usr/bin/env zsh
# tests/zsh/render_in_process.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Zsh prompt renders in the interactive shell: __gpy_render_prompt writes
# __gpy_render_out and __gpy_render_tail instead of printing, so memo writes
# land in the shell and no relay file is needed.
#   - the render function itself runs in the shell's own process;
#   - a second prompt in the same directory reuses the directory memo;
#   - no gpy_relay_* path (or anything else) appears under TMPDIR;
#   - a SIGURG repaint with zle active prints nothing to stderr.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-inproc.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.zsh" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
mkdir -p "$SANDBOX/tmp"
export TMPDIR="$SANDBOX/tmp"

source zsh/gpy.zsh

__color_directory_bg="black" __color_directory_fg="white"
__icon_prompt="❯" __prompt_color="cyan"
GPY_SHOW_STATUS=0
__gpy_two_line=0 __gpy_add_newline=0
__gpy_is_root=0
__gpy_cfg_transient_prompt=1
__gpy_theme_name="inproc"
__enabled_segments=(directory)

# A stub agent: counts `directory` requests in a file (segments run in `$(...)`).
DIR_LOG="$SANDBOX/dir-requests"
: >"$DIR_LOG"
function __gpy_request() {
    [[ $1 == directory ]] && print -r -- x >>"$DIR_LOG"
    printf 'DIR'
}
function __gpy_request_character() { printf 'CHAR'; }

mkdir -p "$SANDBOX/w"
cd "$SANDBOX/w" || exit 1

# --- no fork for the render itself ------------------------------------------------
functions[__gpy_render_prompt_real]=${functions[__gpy_render_prompt]}
typeset -g render_pid=""
function __gpy_render_prompt() {
    render_pid=$sysparams[pid]
    __gpy_render_prompt_real "$@"
}
my_pid=$sysparams[pid]
__gpy_update_prompts 0
check "the render runs in the shell's own process (sysparams[pid] == \$\$)" "$$" "$render_pid"
check "the shell's own sysparams[pid] equals \$\$" "$$" "$my_pid"
[[ "$PROMPT" == *DIR*CHAR* ]]
check "PROMPT holds the segment and the symbol" 0 $?
check "the render tail is the symbol alone" "CHAR" "$__gpy_render_tail"

# --- the directory memo is reused --------------------------------------------------
: >"$DIR_LOG"
__gpy_dir_cache_key="" __gpy_dir_cache_val=""
__gpy_update_prompts 0
__gpy_update_prompts 0
check "two prompts in one directory send one directory request" 1 "$(shell_e2e_count_lines "$DIR_LOG")"
cd "$SANDBOX" || exit 1
__gpy_update_prompts 0
check "another directory asks again" 2 "$(shell_e2e_count_lines "$DIR_LOG")"
cd "$SANDBOX/w" || exit 1

# --- nothing under TMPDIR ----------------------------------------------------------
local_entries=("$TMPDIR"/*(N) "$TMPDIR"/.*(N))
check "no gpy_relay_* (or any other) entry under TMPDIR" 0 "${#local_entries}"
check "__gpy_cache_dir does not exist" 0 "${+__gpy_cache_dir}"

# --- SIGURG repaint with zle active ------------------------------------------------
function zle() { return 0; }
err=$({ TRAPURG; } 2>&1 >/dev/null)
check "a repaint with zle active prints nothing on stderr" "" "$err"
unfunction zle

test_finish "in-process render"
