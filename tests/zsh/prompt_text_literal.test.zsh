#!/usr/bin/env zsh
# tests/zsh/prompt_text_literal.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #677 (twin of tests/bash/prompt_text_literal.test.bash):
# agent-rendered segment text lands in PROMPT as prompt source, and zsh (with
# the prompt_subst gpy sets) expands `$(...)`, `$VAR`, backticks and `%x`
# prompt escapes in it on every draw. Directory and branch names are
# untrusted data, so this ran commands.
#
# With a live sandboxed agent, render the real prompt in directories and on a
# branch whose names contain those characters, and assert that
# `print -rP -- "$PROMPT"` (zsh's own prompt expansion, under the options gpy
# pins) shows them literally: for a fresh IPC render, an instant-cache hit
# and a oneshot fallback.

ROOT=${0:a:h:h:h}
emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
failures=0
fail() { print -r -- "FAIL: $*"; failures=$((failures + 1)); }
pass() { print -r -- "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/zsh/gpy.zsh"

# What zsh displays for prompt source $1, with ANSI SGR removed.
shown() {
    print -rP -- "$1" | python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'
}

# check LABEL SOURCE LITERAL: SOURCE must display LITERAL verbatim.
check() {
    local label=$1 source=$2 literal=$3 display
    display=$(shown "$source")
    if [[ "$display" == *"$literal"* ]]; then
        pass "$label shows '$literal' literally"
    else
        fail "$label: expected '$literal' in displayed prompt, got '$display'"
    fi
}

names=('$(echo GPY_EXPANDED)' '100%_done' 'a\u\hb' 'x!y' 'tick`echo GPY_TICK`')

# --- Fresh IPC render: directory segment ---
__enabled_segments=(directory)
for name in "${names[@]}"; do
    dir="$SHELL_E2E_ROOT/w/$name"
    mkdir -p "$dir"
    cd "$dir" || exit 1
    PROMPT=$(__gpy_render_prompt 0)
    check "IPC directory render" "$PROMPT" "$name"
done

# --- Oneshot fallback render: directory segment ---
dir="$SHELL_E2E_ROOT/w/\$(echo GPY_EXPANDED)"
cd "$dir" || exit 1
__gpy_oneshot_used=0
oneshot_out=$(__gpy_fallback_oneshot directory "$PWD" zsh-prompt true "")
check "oneshot directory render" "$oneshot_out" '$(echo GPY_EXPANDED)'

# --- Git branch: fresh IPC render, then instant-cache hit ---
branch='feat/$HOME-100%_x'
cd "$SHELL_E2E_REPO" || exit 1
git checkout -q -b "$branch"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"

# A fresh IPC render (no cache file to serve) once the agent has seen the
# checkout. The lone git segment is both first and last.
ipc_git=""
ipc_git_shows_branch() {
    rm -f "$cache_dir"/*(N)
    ipc_git=$(__gpy_request git "$SHELL_E2E_REPO" zsh-prompt true "" true)
    [[ "$(shown "$ipc_git")" == *"$branch"* ]]
}
shell_e2e_poll 5 ipc_git_shows_branch
check "IPC git render" "$ipc_git" "$branch"

suffix=$(__gpy_cache_variant_suffix git true true)
resolved_root=${SHELL_E2E_REPO:A}
cache_file="$cache_dir/$(__gpy_path_to_cache_key "$resolved_root").$suffix.none.zsh"
if shell_e2e_poll 3 test -f "$cache_file"; then
    cached=$(__gpy_read_instant_cache "$suffix" "$SHELL_E2E_REPO" "")
    check "instant-cache git hit" "$cached" "$branch"
    __enabled_segments=(git)
    PROMPT=$(__gpy_render_prompt 0)
    check "cached git render" "$PROMPT" "$branch"
else
    fail "the agent did not write the zsh-dialect git cache file $cache_file"
fi

shell_e2e_stop_agent

if [ "$failures" -ne 0 ]; then
    echo "=== $failures FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
