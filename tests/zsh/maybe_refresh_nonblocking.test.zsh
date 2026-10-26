#!/usr/bin/env zsh
# tests/zsh/maybe_refresh_nonblocking.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_maybe_refresh: throttled, non-blocking background refresh (the Zsh
# twin of Fish's __gpy_maybe_refresh).
#
# A stale cache entry with the agent up must not send a background request on
# EVERY render: __gpy_maybe_refresh caps that at one request per
# 500 ms per op+suffix+path(+token). Segments render inside `$(...)` subshells,
# so the throttle state must survive them (it lives in a per-shell stamp file).
# Asserted with a recording stub for __gpy_trigger_data_refresh and a real agent:
#   (a) the call returns at once even when the refresh itself takes seconds;
#   (b) a burst of calls, including from separate subshells, sends ONE request;
#   (c) a different throttle key (variant token) is not starved by (b);
#   (d) after the interval the same key sends again;
#   (e) a stale git segment render burst sends ONE request;
#   (f) with the agent disabled nothing is sent at all.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"

shell_e2e_text_config '"git"'
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (the theme export would re-set the
# flag from config.toml) so sourcing starts no daemon; enable it afterwards.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/zsh/gpy.zsh"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1

calls="$SHELL_E2E_ROOT/refresh-calls"
: >"$calls"
# Recording stub; the delay makes a blocking dispatcher observable.
stub_delay=1
__gpy_trigger_data_refresh() { sleep "$stub_delay"; echo "$1 $2" >>"$calls"; }
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
if shell_e2e_poll 5 shell_e2e_has_lines "$calls" 1; then
    # Window for a second (throttle-violating) request to land; nothing to poll for.
    sleep "$SHELL_E2E_ABSENCE_WINDOW"
    shell_e2e_has_lines "$calls" 1 && pass "a burst across the shell and subshells sent exactly one request" \
        || fail "burst sent $(shell_e2e_count_lines "$calls") requests"
else
    fail "the first call never reached the refresh (count $(shell_e2e_count_lines "$calls"))"
fi

# --- (c) a different throttle key is independent -----------------------------------------
__gpy_maybe_refresh git "$root" git_last true black "" 222
if shell_e2e_poll 5 shell_e2e_has_lines "$calls" 2; then
    pass "a different variant token is throttled independently"
else
    fail "the variant-token refresh was starved (count $(shell_e2e_count_lines "$calls"))"
fi

# --- (d) after the interval the same key sends again ------------------------------------
# The throttle reads time through __gpy_now_ms: move a fake clock past the
# 500 ms interval instead of sleeping it out.
__gpy_now_ms fake_ms
__gpy_now_ms() { printf -v "$1" '%s' "$fake_ms"; }
advance_clock_past_throttle() { fake_ms=$((fake_ms + 600)); }
advance_clock_past_throttle
__gpy_maybe_refresh git "$root" git_last true black ""
if shell_e2e_poll 5 shell_e2e_has_lines "$calls" 3; then
    pass "after the throttle interval the same key sends again"
else
    fail "no refresh after the interval (count $(shell_e2e_count_lines "$calls"))"
fi

# --- (e) a stale git segment burst sends one request --------------------------------------
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"
__gpy_path_to_cache_key "$(realpath "$root")" key
# `red`, not the theme's directory background: a config reload makes the agent
# re-render the git entries for that background, which would overwrite the
# sentinel mid-test.
entry="$cache_dir/$key.git_last.red.zsh"
printf '%s' "STALE" >"$entry"
python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-60, time.time()-60))" "$entry"
# Wait out the 500 ms throttle window left by (d).
advance_clock_past_throttle
before="$(shell_e2e_count_lines "$calls")"
for _ in 1 2 3 4; do out="$(__gpy_segment_git "true" red "")"; done
if [ "$out" = "STALE" ]; then
    pass "the stale value is still served immediately"
else
    fail "expected the stale value, got '$out'"
fi
if shell_e2e_poll 5 shell_e2e_has_lines "$calls" $((before + 1)); then
    # Window for a second (throttle-violating) request to land; nothing to poll for.
    sleep "$SHELL_E2E_ABSENCE_WINDOW"
    shell_e2e_has_lines "$calls" $((before + 1)) && pass "four stale renders sent exactly one request" \
        || fail "four stale renders sent $(($(shell_e2e_count_lines "$calls") - before)) requests"
else
    fail "the stale renders sent no request (count $(shell_e2e_count_lines "$calls"), before $before)"
fi

# --- (f) agent-free mode: nothing is sent ------------------------------------------------
advance_clock_past_throttle # so a missing request below is not explained by the throttle
before="$(shell_e2e_count_lines "$calls")"
stub_delay=0.1
export GPY_AGENT_ENABLED=0
__gpy_maybe_refresh git "$root" git_last true black ""
# Absence cannot be polled for: a fixed window, several times the stub's delay,
# in which any refresh that was sent would have been recorded.
sleep "$SHELL_E2E_ABSENCE_WINDOW"
export GPY_AGENT_ENABLED=1
shell_e2e_has_lines "$calls" "$before" && pass "with the agent disabled no refresh is sent" \
    || fail "a refresh was sent with GPY_AGENT_ENABLED=0"

test_finish "__gpy_maybe_refresh is throttled and non-blocking"
