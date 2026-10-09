#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
# tests/bash/maybe_refresh_nonblocking.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_maybe_refresh: throttled, non-blocking background refresh (#843; the
# Bash twin of Fish's __gpy_maybe_refresh, #612).
#
# A stale cache entry with the agent up used to send an unthrottled background
# request on EVERY render. __gpy_maybe_refresh now caps that at one request per
# 500 ms per op+suffix+path(+token). Segments render inside `$(...)` subshells,
# so the throttle state must survive them (it lives in a per-shell stamp file).
# Asserted with a recording stub for __gpy_trigger_data_refresh and a real agent:
#   (a) the call returns at once even when the refresh itself takes seconds;
#   (b) a burst of calls, including from separate subshells, sends ONE request;
#   (c) a different throttle key (variant token) is not starved by (b);
#   (d) after the interval the same key sends again;
#   (e) a stale git segment render burst sends ONE request;
#   (f) with the agent disabled (#841) nothing is sent at all.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (the theme export would re-set the
# flag from config.toml) so sourcing starts no daemon; enable it afterwards.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/bash/gpy.bash"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1

calls="$SHELL_E2E_ROOT/refresh-calls"
: >"$calls"
# Recording stub; the sleep makes a blocking dispatcher observable.
__gpy_trigger_data_refresh() { sleep 1; echo "$1 $2" >>"$calls"; }
count() { wc -l <"$calls" | tr -d ' '; }
count_is() { [ "$(count)" -eq "$1" ]; }
now_s() { python3 -c 'import time; print(time.time())'; }
root="$SHELL_E2E_REPO"

# --- (a)+(b) one request per burst, and the dispatcher never blocks ----------------------
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
    sleep 0.3
    [ "$(count)" -eq 1 ] && pass "a burst across the shell and subshells sent exactly one request" \
        || fail "burst sent $(count) requests"
else
    fail "the first call never reached the refresh (count $(count))"
fi

# --- (c) a different throttle key is independent -----------------------------------------
__gpy_maybe_refresh git "$root" git_last true black "" 222
if shell_e2e_poll 5 count_is 2; then
    pass "a different variant token is throttled independently"
else
    fail "the variant-token refresh was starved (count $(count))"
fi

# --- (d) after the interval the same key sends again ------------------------------------
sleep 1.2
__gpy_maybe_refresh git "$root" git_last true black ""
if shell_e2e_poll 5 count_is 3; then
    pass "after the throttle interval the same key sends again"
else
    fail "no refresh after the interval (count $(count))"
fi

# --- (e) a stale git segment burst sends one request --------------------------------------
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"
key="$(__gpy_path_to_cache_key "$(realpath "$root")")"
# `red`, not the theme's directory background: a config reload makes the agent
# re-render the git entries for that background, which would overwrite the
# sentinel mid-test.
entry="$cache_dir/$key.git_last.red.bash"
printf '%s' "STALE" >"$entry"
python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-60, time.time()-60))" "$entry"
sleep 0.6
before="$(count)"
for _ in 1 2 3 4; do out="$(__gpy_segment_git "true" red "")"; done
if [ "$out" = "STALE" ]; then
    pass "the stale value is still served immediately"
else
    fail "expected the stale value, got '$out'"
fi
if shell_e2e_poll 5 count_is $((before + 1)); then
    sleep 0.3
    [ "$(count)" -eq $((before + 1)) ] && pass "four stale renders sent exactly one request" \
        || fail "four stale renders sent $(($(count) - before)) requests"
else
    fail "the stale renders sent no request (count $(count), before $before)"
fi

# --- (f) agent-free mode: nothing is sent ------------------------------------------------
sleep 1.2
before="$(count)"
export GPY_AGENT_ENABLED=0
__gpy_maybe_refresh git "$root" git_last true black ""
sleep 1.3
export GPY_AGENT_ENABLED=1
[ "$(count)" -eq "$before" ] && pass "with the agent disabled no refresh is sent" \
    || fail "a refresh was sent with GPY_AGENT_ENABLED=0"

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: __gpy_maybe_refresh is throttled and non-blocking"
