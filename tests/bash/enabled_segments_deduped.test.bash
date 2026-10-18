#!/usr/bin/env bash
# tests/bash/enabled_segments_deduped.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# A ui.enabled_segments entry repeated in any spelling keeps only its first
# occurrence, so the exported list and the prompt carry each segment once.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"git", "Directory", "Git", " git ", "directory"' false supervisor-off
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0

stored="$("$ROOT/gpy-agent/target/debug/gpy" config get ui.enabled_segments 2>&1)"
if [[ "$stored" == "git directory" ]]; then
    pass "gpy config get reports each segment once, first occurrence first"
else
    fail "gpy config get ui.enabled_segments: expected 'git directory', got '$stored'"
fi

shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
# shellcheck source=bash/gpy.bash
source "$ROOT/bash/gpy.bash"

# shellcheck disable=SC2154 # set by the theme export gpy.bash sources
if [[ "$__enabled_segments" == "git directory" ]]; then
    pass "theme export hands Bash each segment once"
else
    fail "__enabled_segments: expected 'git directory', got '$__enabled_segments'"
fi

# Occurrences of $2 in $1.
count_of() {
    local rest="$1" n=0
    while [[ "$rest" == *"$2"* ]]; do
        rest="${rest#*"$2"}"
        n=$((n + 1))
    done
    printf '%s\n' "$n"
}

# The git segment may cold-miss on the first draw; re-render until it lands.
git_rendered() {
    __gpy_render_prompt 0
    case "$PS1" in *main*) return 0 ;; esac
    return 1
}
if shell_e2e_poll 5 git_rendered; then
    pass "git segment renders in the prompt"
else
    fail "git segment missing from PS1: '$PS1'"
fi

branch_count="$(count_of "$PS1" main)"
if [[ "$branch_count" == 1 ]]; then
    pass "git segment renders once"
else
    fail "git segment rendered $branch_count times: '$PS1'"
fi

repo_name="$(basename "$SHELL_E2E_REPO")"
dir_count="$(count_of "$PS1" "$repo_name")"
if [[ "$dir_count" == 1 ]]; then
    pass "directory segment renders once"
else
    fail "directory segment rendered $dir_count times: '$PS1'"
fi

test_finish "repeated enabled segments render once"
