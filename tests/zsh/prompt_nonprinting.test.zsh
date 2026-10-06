#!/usr/bin/env zsh
# tests/zsh/prompt_nonprinting.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #679 (twin of tests/bash/prompt_nonprinting.test.bash):
# agent-rendered SGR escapes reached PROMPT without zsh's `%{ %}` zero-width
# markers, so ZLE counted every escape byte as a printed column. Typed input
# wrapped early and Ctrl-A put the cursor on the wrong row.
#
# With a live sandboxed agent and the default theme, render the real prompt
# and assert that once every `%{...%}` region is removed from PROMPT, no ESC
# byte is left. Covers a fresh IPC render and the instant-cache hit that
# follows it.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
failures=0
fail() { print -r -- "FAIL: $*"; failures=$((failures + 1)); }
pass() { print -r -- "PASS: $*"; }

printf '[ui]\ntheme = "default"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/zsh/gpy.zsh"

# PROMPT with every `%{...%}` region removed: the bytes ZLE counts.
printed() {
    print -rn -- "$1" | python3 -c '
import re, sys
sys.stdout.write(re.sub(r"%\{.*?%\}", "", sys.stdin.read(), flags=re.S))'
}

check_prompt() {
    local label=$1 visible
    if [[ "$PROMPT" != *$'\e'* ]]; then
        fail "$label: PROMPT carries no SGR at all, so nothing was checked: ${(q+)PROMPT}"
        return
    fi
    if [[ "$PROMPT" != *repo* ]]; then
        fail "$label: the agent directory segment is missing: ${(q+)PROMPT}"
        return
    fi
    visible=$(printed "$PROMPT")
    if [[ "$visible" == *$'\e'* ]]; then
        fail "$label: an escape sits outside %{ %}: ${(q+)visible}"
    else
        pass "$label: every escape is inside %{ %}"
    fi
}

__enabled_segments=(directory git)
PROMPT=$(__gpy_render_prompt 0)
check_prompt "fresh render"

# The render above warmed the zsh-dialect instant cache; render again so the
# git segment is served from it.
cache_written() {
    local -a files
    files=("$XDG_CACHE_HOME"/gpy/instant-prompts/*.git_last.*.zsh(N))
    (( ${#files} > 0 ))
}
if shell_e2e_poll 3 cache_written; then
    PROMPT=$(__gpy_render_prompt 0)
    check_prompt "instant-cache render"
else
    fail "the agent did not write a zsh-dialect git cache file"
fi

shell_e2e_stop_agent

if [ "$failures" -ne 0 ]; then
    echo "=== $failures FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
