#!/usr/bin/env zsh
# tests/zsh/e2e_git_cold_miss_bounded_sync.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Git segment cold miss: bounded synchronous IPC, never a foreground oneshot
# (#843; the Zsh twin of tests/fish/e2e_git_cold_miss_bounded_sync.test.fish,
# #434).
#
# Before this fix a cold instant-cache miss fell through to __gpy_request,
# which runs a foreground `gpy-agent oneshot git` when the agent is down --
# unbounded, and unlike Fish, which omits the segment.
#   (a) agent DOWN: the render omits the segment (empty, status 0), instantly,
#       and never forks `gpy-agent oneshot` (a recording stub proves it);
#   (b) agent UP: the very first render of a never-queried repo returns real,
#       non-empty git output from one bounded synchronous IPC round-trip.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (the theme export would re-set the
# flag from config.toml) so sourcing starts no daemon, then enable it: the
# segments read GPY_AGENT_ENABLED at call time.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/zsh/gpy.zsh"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0

strip() { python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'; }
oneshot_log="$SHELL_E2E_ROOT/oneshot-calls"
# A shell function shadows the binary for `gpy-agent oneshot ...`; any call
# the segment makes is recorded here.
gpy-agent() { echo "$*" >>"$oneshot_log"; }
cache_files() { find "$XDG_CACHE_HOME/gpy/instant-prompts" -maxdepth 1 -name '*.git.*.zsh' 2>/dev/null | head -n 1; }

# --- (a) agent down: omit, no oneshot --------------------------------------------
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

# --- (b) agent up: bounded synchronous render ----------------------------------------
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

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh git segment cold miss is a bounded sync query, never a foreground oneshot"
