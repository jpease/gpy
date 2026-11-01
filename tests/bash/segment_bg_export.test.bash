#!/usr/bin/env bash
# tests/bash/segment_bg_export.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_segment_bg comes from the debug agent's `theme export`: its git arm
# returns the background the agent recorded for the repository, and the clean
# colour when there is none, on agent-free renders, or when the helper is absent.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

AGENT_BIN="$ROOT/gpy-agent/target/debug/gpy-agent"
if [[ ! -x "$AGENT_BIN" ]]; then
    test_skip "$AGENT_BIN not built (run \`cargo build\` in gpy-agent first)"
fi

source bash/core/constants.bash
source bash/core/ipc.bash

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-segbg.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
export XDG_CACHE_HOME="$SANDBOX/cache"

export_output="$("$AGENT_BIN" theme export --shell bash 2>/dev/null)"
if [[ -z "$export_output" ]]; then
    echo "FAIL: gpy-agent theme export --shell bash produced no output"
    exit 1
fi
eval "$export_output"

check "__gpy_segment_bg git matches \$__color_git_clean_bg" \
    "${__color_git_clean_bg:-}" "$(__gpy_segment_bg git)"

# Fork-free out-var calling convention (the render loop's hot path).
out_var_result=""
__gpy_segment_bg git out_var_result
check "__gpy_segment_bg git OUT_VAR writes the same value" \
    "${__color_git_clean_bg:-}" "$out_var_result"

check "unknown segment returns empty" "" "$(__gpy_segment_bg not-a-real-segment)"

# A recorded colour reaches the render loop through the generated function.
repo="$SANDBOX/repo"
mkdir -p "$repo/.git"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts" key="" bg_tail=""
__gpy_path_to_cache_key "$(realpath "$repo")" key
__gpy_git_bg_file_tail bash bg_tail
mkdir -p "$cache_dir"
bg_file="$cache_dir/$key.$bg_tail"
cd "$repo" || exit 1
__gpy_resolve_pwd

check "no git_bg file: the clean colour" \
    "${__color_git_clean_bg:-}" "$(__gpy_segment_bg git)"

printf 'red' >"$bg_file"
check "git_bg file: the recorded colour" red "$(__gpy_segment_bg git)"
out_var_result=""
__gpy_segment_bg git out_var_result
check "git_bg file: OUT_VAR form" red "$out_var_result"

__gpy_git_seg_status="$__gpy_seg_status_oneshot"
check "agent-free git render ignores the file" \
    "${__color_git_clean_bg:-}" "$(__gpy_segment_bg git)"
unset __gpy_git_seg_status

forks="$SANDBOX/forks"
: >"$forks"
saved_debug_trap="$(trap -p DEBUG)"
set -T
out_var_result=""
trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "${FUNCNAME[0]}: $BASH_COMMAND" >>"$forks"' DEBUG
__gpy_segment_bg git out_var_result
trap - DEBUG
set +T
eval "$saved_debug_trap"
check "fork check reads the recorded colour" red "$out_var_result"
check "bash $BASH_VERSION: reading the colour runs no subshell" "" "$(<"$forks")"

unset -f __gpy_git_segment_bg
err_output="$(__gpy_segment_bg git 2>&1)"
check "without the helper: the clean colour, no error" \
    "${__color_git_clean_bg:-}" "$err_output"

cd "$ROOT" || exit 1
test_finish "segment_bg export test"
