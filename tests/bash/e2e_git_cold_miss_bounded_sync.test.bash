#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
# tests/bash/e2e_git_cold_miss_bounded_sync.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# A cold miss must not fall through to a foreground oneshot: (a) agent down omits the
# segment without forking; (b) agent up returns real output via one bounded sync IPC.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (no daemon), then enable it.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/bash/gpy.bash"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0

strip() { python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'; }
oneshot_log="$SHELL_E2E_ROOT/oneshot-calls"
# Shell function shadowing `gpy-agent oneshot`; records any call.
gpy-agent() { echo "$*" >>"$oneshot_log"; }
cache_files() { find "$XDG_CACHE_HOME/gpy/instant-prompts" -maxdepth 1 -name '*.git.*.bash' 2>/dev/null | head -n 1; }

echo "--- (a) cold miss, agent down ---"
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "a socket exists; this test needs the agent down"
[ -z "$(cache_files)" ] || fail "the instant cache is not cold before the render"
out="$(__gpy_segment_git "true" "" "")"
rc=$?
if [ -z "$out" ] && [ "$rc" -eq 0 ]; then
    pass "the cold-miss render omits the segment with status 0"
else
    fail "expected an empty render with status 0, got '$out' status $rc"
fi
if [ ! -e "$oneshot_log" ]; then
    pass "no foreground gpy-agent oneshot was forked"
else
    fail "the cold-miss render forked gpy-agent oneshot: $(cat "$oneshot_log")"
fi
unset -f gpy-agent

echo "--- (b) cold miss, agent up ---"
shell_e2e_start_agent || exit 1
[ -z "$(cache_files)" ] || fail "the instant cache is not cold before the agent-up render"
out="$(__gpy_segment_git "true" "" "")"
rc=$?
text="$(printf '%s' "$out" | strip)"
case "$text" in
    *main*) pass "the first render on a cold cache shows the branch (bounded sync IPC)" ;;
    *) fail "the first render on a cold cache was '$text' (status $rc)" ;;
esac

test_finish "bash git segment cold miss is a bounded sync query, never a foreground oneshot"
