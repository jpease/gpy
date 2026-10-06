#!/usr/bin/env zsh
# tests/zsh/e2e_agent_down_stale_git.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A stale git segment is never served forever when the agent is down (#647
# row 4, pins #639).
#
# #430 gave the Fish git segment a bounded oneshot recovery: on a stale
# instant-cache hit with no socket, run `gpy-agent oneshot git` in the
# foreground so the prompt shows the current state. Zsh kept serving the
# cached segment and fired a background refresh at a socket nobody listened
# on, so a user whose agent died saw the last cached state indefinitely.
#
# No pty needed. With the real gpy-agent on PATH and a dead socket:
#   (a) a STALE cache entry holding a sentinel plus a dirty tree renders the
#       real oneshot output (branch + unstaged marker), not the sentinel;
#   (b) the segment reports the oneshot budget claim (GPY_SEG_STATUS_ONESHOT)
#       so a render can never fork more than one oneshot;
#   (c) a FRESH cache entry is still served as-is (no fork);
#   (d) with the socket present (agent up), a stale entry is served as-is and
#       refreshed in the background, exactly as before.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/zsh/gpy.zsh"

backdate() {
    python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-60, time.time()-60))" "$1"
}

resolved_root="$(realpath "$SHELL_E2E_REPO" 2>/dev/null || echo "$SHELL_E2E_REPO")"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"
key="$(__gpy_path_to_cache_key "$resolved_root")"
# `__gpy_segment_git "true" black ""`: is_last=true, prev_bg=black, is_first="".
entry="$cache_dir/$key.git_last.black.zsh"
sentinel="STALE-SENTINEL-DO-NOT-SHOW"

strip() { python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'; }

# --- (a) stale entry, dirty tree, agent down -----------------------------------
echo "--- (a) stale cache, agent down ---"
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "a socket exists; this test needs the agent down"
echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
printf '%s' "$sentinel" >"$entry"
backdate "$entry"

out="$(__gpy_segment_git "true" black "")"
rc=$?
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
if [ "$rc" -eq "${GPY_SEG_STATUS_ONESHOT:-3}" ]; then
    pass "the segment reported the oneshot budget claim (status $rc)"
else
    fail "expected exit status GPY_SEG_STATUS_ONESHOT (${GPY_SEG_STATUS_ONESHOT:-3}), got $status"
fi

# --- (b) budget: a second call in the same render does not fork again ------------
echo "--- (b) oneshot budget ---"
__gpy_oneshot_used=1
out2="$(__gpy_segment_git "true" black "")"
rc2=$?
text2="$(printf '%s' "$out2" | strip)"
if [ "$rc2" -ne "${GPY_SEG_STATUS_ONESHOT:-3}" ]; then
    pass "with the budget spent the segment does not fork oneshot again (status $rc2)"
else
    fail "the segment forked a second oneshot in one render"
fi
case "$text2" in
    *"$sentinel"*) pass "with the budget spent the stale value is served rather than nothing" ;;
    *) fail "expected the stale value once the budget is spent, got '$text2'" ;;
esac
unset __gpy_oneshot_used

# --- (c) a fresh entry is served as-is, no fork ---------------------------------------
echo "--- (c) fresh cache ---"
printf '%s' "FRESH-VALUE" >"$entry"
out3="$(__gpy_segment_git "true" black "")"
rc3=$?
if [ "$out3" = "FRESH-VALUE" ] && [ "$rc3" -eq 0 ]; then
    pass "a fresh entry is served verbatim with status 0"
else
    fail "fresh entry mis-served: '$out3' status $rc3"
fi

# --- (d) agent up: stale is served and refreshed in the background -------------------
echo "--- (d) stale cache, agent up ---"
shell_e2e_start_agent || exit 1
printf '%s' "$sentinel" >"$entry"
backdate "$entry"
out4="$(__gpy_segment_git "true" black "")"
rc4=$?
case "$out4" in
    "$sentinel") pass "with the agent up the stale value is served immediately (status $rc4)" ;;
    *) fail "with the agent up expected the stale value, got '$out4'" ;;
esac
refreshed() { [ -f "$entry" ] && ! grep -q "$sentinel" "$entry"; }
if shell_e2e_poll 10 refreshed; then
    pass "the background refresh rewrote the stale entry"
else
    fail "the stale entry was not refreshed within 10 s with the agent up"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh git segment recovers from a stale cache when the agent is down"
