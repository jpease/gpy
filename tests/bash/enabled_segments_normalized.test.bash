#!/usr/bin/env bash
# tests/bash/enabled_segments_normalized.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# ui.enabled_segments entries are stored trimmed and lowercased, so a config
# spelling `Directory` / ` git ` loads, exports and renders those segments.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"Directory", " git "' false supervisor-off
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0

stored="$("$ROOT/gpy-agent/target/debug/gpy" config get ui.enabled_segments 2>&1)"
if [[ "$stored" == "directory git" ]]; then
    pass "gpy config get reports canonical names"
else
    fail "gpy config get ui.enabled_segments: expected 'directory git', got '$stored'"
fi

shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
# shellcheck source=bash/gpy.bash
source "$ROOT/bash/gpy.bash"

# shellcheck disable=SC2154 # set by the theme export gpy.bash sources
if [[ "$__enabled_segments" == "directory git" ]]; then
    pass "theme export hands Bash canonical segment names"
else
    fail "__enabled_segments: expected 'directory git', got '$__enabled_segments'"
fi

__gpy_render_prompt 0
repo_name="$(basename "$SHELL_E2E_REPO")"
case "$PS1" in
    *"$repo_name"*) pass "directory segment renders in the prompt" ;;
    *) fail "directory segment missing from PS1: '$PS1'" ;;
esac
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

test_finish "enabled segment names are normalized end to end"
