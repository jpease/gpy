#!/usr/bin/env bash
# tests/bash/prompt_nonprinting.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Agent SGR escapes must sit inside PS1 `\[ \]` markers or readline miscounts
# columns. Checks a fresh IPC render and the following instant-cache hit.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

printf '[ui]\ntheme = "default"\nenabled_segments = ["directory", "git"]\n\n[agent.supervisor]\nenabled = false\n' \
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

# Render again: served from the instant cache.
cache_glob="$XDG_CACHE_HOME/gpy/instant-prompts/*.git_last.*.bash"
cache_written() { compgen -G "$cache_glob" >/dev/null; }
if shell_e2e_poll 3 cache_written; then
    __gpy_render_prompt 0
    check_prompt "instant-cache render"
else
    fail "the agent did not write a bash-dialect git cache file ($cache_glob)"
fi

shell_e2e_stop_agent

test_finish
