#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
# tests/bash/maybe_refresh_nonblocking.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_maybe_refresh caps background refreshes at one per 500 ms per
# op+suffix+path(+token); state lives in a stamp file to survive `$(...)` subshells.
#   (a) the call returns at once even when the refresh itself takes seconds;
#   (b) a burst of calls, including from separate subshells, sends ONE request;
#   (c) a different throttle key (variant token) is not starved by (b);
#   (d) after the interval the same key sends again;
#   (e) a stale git segment render burst sends ONE request;
#   (f) with the agent disabled nothing is sent at all.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"git"'
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH so no daemon starts; enable afterwards.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/bash/gpy.bash"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1

calls="$SHELL_E2E_ROOT/refresh-calls"
: >"$calls"
stub_delay=1
__gpy_trigger_data_refresh() { sleep "$stub_delay"; echo "$1 $2" >>"$calls"; }
count() { wc -l <"$calls" | tr -d ' '; }
count_is() { [ "$(count)" -eq "$1" ]; }
now_s() { python3 -c 'import time; print(time.time())'; }
root="$SHELL_E2E_REPO"

t0="$(now_s)"
__gpy_maybe_refresh git "$root" git_last true black ""
( __gpy_maybe_refresh git "$root" git_last true black "" )
( __gpy_maybe_refresh git "$root" git_last true black "" )
__gpy_maybe_refresh git "$root" git_last true black ""
t1="$(now_s)"
if python3 -c "import sys; sys.exit(0 if $t1 - $t0 < 0.9 else 1)"; then
    pass "four calls returned without waiting on the 1 s refresh"
else
    fail "__gpy_maybe_refresh blocked on the refresh ($t0 -> $t1)"
fi
if shell_e2e_poll 5 count_is 1; then
    sleep 0.3 # window in which a second request would show up
    [ "$(count)" -eq 1 ] && pass "a burst across the shell and subshells sent exactly one request" \
        || fail "burst sent $(count) requests"
else
    fail "the first call never reached the refresh (count $(count))"
fi

__gpy_maybe_refresh git "$root" git_last true black "" 222
if shell_e2e_poll 5 count_is 2; then
    pass "a different variant token is throttled independently"
else
    fail "the variant-token refresh was starved (count $(count))"
fi

# Move a fake __gpy_now_ms clock past the interval instead of sleeping.
__gpy_now_ms fake_ms
__gpy_now_ms() { printf -v "$1" '%s' "$fake_ms"; }
advance_clock_past_throttle() { fake_ms=$((fake_ms + 600)); }
advance_clock_past_throttle
__gpy_maybe_refresh git "$root" git_last true black ""
if shell_e2e_poll 5 count_is 3; then
    pass "after the throttle interval the same key sends again"
else
    fail "no refresh after the interval (count $(count))"
fi

cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"
key=""
__gpy_path_to_cache_key "$(realpath "$root")" key
# `red`, not the directory background: a config reload re-renders that background's git entries.
entry="$cache_dir/$key.git_last.red.bash"
printf '%s' "STALE" >"$entry"
python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-60, time.time()-60))" "$entry"
advance_clock_past_throttle
before="$(count)"
for _ in 1 2 3 4; do out="$(__gpy_segment_git "true" red "")"; done
if [ "$out" = "STALE" ]; then
    pass "the stale value is still served immediately"
else
    fail "expected the stale value, got '$out'"
fi
if shell_e2e_poll 5 count_is $((before + 1)); then
    sleep 0.3 # window in which a second request would show up
    [ "$(count)" -eq $((before + 1)) ] && pass "four stale renders sent exactly one request" \
        || fail "four stale renders sent $(($(count) - before)) requests"
else
    fail "the stale renders sent no request (count $(count), before $before)"
fi

advance_clock_past_throttle # so a missing request below is not explained by the throttle
before="$(count)"
stub_delay=0.1
export GPY_AGENT_ENABLED=0
__gpy_maybe_refresh git "$root" git_last true black ""
# Absence cannot be polled: wait several stub delays.
sleep 0.5
export GPY_AGENT_ENABLED=1
[ "$(count)" -eq "$before" ] && pass "with the agent disabled no refresh is sent" \
    || fail "a refresh was sent with GPY_AGENT_ENABLED=0"

test_finish "__gpy_maybe_refresh is throttled and non-blocking"
