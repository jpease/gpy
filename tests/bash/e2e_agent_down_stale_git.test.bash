#!/usr/bin/env bash
# tests/bash/e2e_agent_down_stale_git.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A stale git segment is never served forever when the agent is down.
#
# Agent down + stale cache hit: Bash runs `gpy-agent oneshot git` in the foreground
# so a dead agent never pins the last cached state. Also: (b) oneshot budget claim,
# (c) fresh entries served as-is, (d) agent up: stale served + background refresh.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

# Supervisor off at startup and every prompt (theme export re-sets it): agent must be down.
shell_e2e_text_config '"directory", "git"' false supervisor-off
# Source with the agent disabled and off PATH so it can't autostart; enable it afterwards.
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/bash/gpy.bash"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1

backdate() {
    python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-60, time.time()-60))" "$1"
}

resolved_root="$(realpath "$SHELL_E2E_REPO" 2>/dev/null || echo "$SHELL_E2E_REPO")"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"
key=""
__gpy_path_to_cache_key "$resolved_root" key
# `__gpy_segment_git "true" black ""`: is_last=true, prev_bg=black, is_first="".
entry="$cache_dir/$key.git_last.black.bash"
sentinel="STALE-SENTINEL-DO-NOT-SHOW"

strip() { python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'; }

echo "--- (a) stale cache, agent down ---"
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "a socket exists; this test needs the agent down"
echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
printf '%s' "$sentinel" >"$entry"
backdate "$entry"

out="$(__gpy_segment_git "true" black "")"
status=$?
text="$(printf '%s' "$out" | strip)"
case "$text" in
    *"$sentinel"*) fail "the stale sentinel was served with the agent down: $text" ;;
    *main*) pass "the real oneshot output names the branch instead of the stale sentinel" ;;
    *) fail "unexpected render with the agent down: '$text'" ;;
esac
case "$text" in
    *"✱1"*) pass "the oneshot output shows the dirty tree" ;;
    *) fail "the oneshot output does not show the unstaged marker: '$text'" ;;
esac
if [ "$status" -eq "${GPY_SEG_STATUS_ONESHOT:-3}" ]; then
    pass "the segment reported the oneshot budget claim (status $status)"
else
    fail "expected exit status GPY_SEG_STATUS_ONESHOT (${GPY_SEG_STATUS_ONESHOT:-3}), got $status"
fi

echo "--- (b) oneshot budget ---"
__gpy_oneshot_used=1
out2="$(__gpy_segment_git "true" black "")"
status2=$?
text2="$(printf '%s' "$out2" | strip)"
if [ "$status2" -ne "${GPY_SEG_STATUS_ONESHOT:-3}" ]; then
    pass "with the budget spent the segment does not fork oneshot again (status $status2)"
else
    fail "the segment forked a second oneshot in one render"
fi
case "$text2" in
    *"$sentinel"*) pass "with the budget spent the stale value is served rather than nothing" ;;
    *) fail "expected the stale value once the budget is spent, got '$text2'" ;;
esac
unset __gpy_oneshot_used

echo "--- (c) fresh cache ---"
printf '%s' "FRESH-VALUE" >"$entry"
out3="$(__gpy_segment_git "true" black "")"
status3=$?
if [ "$out3" = "FRESH-VALUE" ] && [ "$status3" -eq 0 ]; then
    pass "a fresh entry is served verbatim with status 0"
else
    fail "fresh entry mis-served: '$out3' status $status3"
fi

echo "--- (d) stale cache, agent up ---"
shell_e2e_start_agent || exit 1
printf '%s' "$sentinel" >"$entry"
backdate "$entry"
out4="$(__gpy_segment_git "true" black "")"
status4=$?
case "$out4" in
    "$sentinel") pass "with the agent up the stale value is served immediately (status $status4)" ;;
    *) fail "with the agent up expected the stale value, got '$out4'" ;;
esac
refreshed() { [ -f "$entry" ] && ! grep -q "$sentinel" "$entry"; }
if shell_e2e_poll 10 refreshed; then
    pass "the background refresh rewrote the stale entry"
else
    fail "the stale entry was not refreshed within 10 s with the agent up"
fi

test_finish "bash git segment recovers from a stale cache when the agent is down"
