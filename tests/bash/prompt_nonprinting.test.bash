#!/usr/bin/env bash
# tests/bash/prompt_nonprinting.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #679: agent-rendered SGR escapes reached PS1 without
# bash's `\[ \]` non-printing markers, so readline counted every escape byte
# as a printed column. Typed input wrapped early and Ctrl-A, history recall
# and completion put the cursor on the wrong row and column.
#
# With a live sandboxed agent and the default theme, render the real prompt
# (agent directory, git and character segments) and assert that once every
# `\[...\]` region is removed from PS1, no ESC byte is left. Covers a fresh
# IPC render and the instant-cache hit that follows it.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\ntheme = "default"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
# shellcheck source=bash/gpy.bash
source "$ROOT/bash/gpy.bash"

# PS1 with every `\[...\]` region removed: the bytes readline counts.
printed() {
    printf '%s' "$1" | python3 -c '
import re, sys
sys.stdout.write(re.sub(r"\\\[.*?\\\]", "", sys.stdin.read(), flags=re.S))'
}

check_prompt() {
    local label="$1" visible
    if [[ "$PS1" != *$'\e'* ]]; then
        fail "$label: PS1 carries no SGR at all, so nothing was checked: $(printf '%q' "$PS1")"
        return
    fi
    if [[ "$PS1" != *repo* ]]; then
        fail "$label: the agent directory segment is missing: $(printf '%q' "$PS1")"
        return
    fi
    visible="$(printed "$PS1")"
    if [[ "$visible" == *$'\e'* ]]; then
        fail "$label: an escape sits outside \\[ \\]: $(printf '%q' "$visible")"
    else
        pass "$label: every escape is inside \\[ \\]"
    fi
}

__enabled_segments="directory git"
__gpy_render_prompt 0
check_prompt "fresh render"

# The render above warmed the bash-dialect instant cache; render again so
# the git segment is served from it.
cache_glob="$XDG_CACHE_HOME/gpy/instant-prompts/*.git_last.*.bash"
cache_written() { compgen -G "$cache_glob" >/dev/null; }
if shell_e2e_poll 3 cache_written; then
    __gpy_render_prompt 0
    check_prompt "instant-cache render"
else
    fail "the agent did not write a bash-dialect git cache file ($cache_glob)"
fi

shell_e2e_stop_agent

if [ "$failures" -ne 0 ]; then
    echo "=== $failures FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
