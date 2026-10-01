#!/usr/bin/env fish

# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# Unit Test: Agent Availability Circuit Breaker Expiry
# ============================================================================
#
# The circuit breaker in `__gpy_agent_available` (fish/core/util.fish) opens
# for GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS after repeated ping failures, which
# is what happens to every open shell while the agent is restarting.
#
# It must CLOSE again once that window elapses. A breaker that latches open
# permanently makes the agent unreachable for the rest of the shell's life:
# `__gpy_register_with_agent` gates on `__gpy_agent_available`, so such a shell
# can never re-register -- not from the agent's restart .reregister nudge, not
# from the agent's re-nudge, not from a prompt render -- and therefore never
# receives another live update (#464).

source (dirname (status -f))/../lib/test_helpers.fish

# Bind to the REPO's util.fish explicitly. Fish sources the user's config.fish
# even for scripts, so without this the test would silently exercise whatever
# gpy version happens to be installed on this machine instead of the tree under
# test.
source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
source $__gpy_root/fish/core/util.fish

set -g __probe_ping_calls 0

# Install a fake agent binary + ping so these tests exercise the breaker logic
# alone, with no dependency on a running agent.
function install_probe_stubs
    functions -e __gpy_resolve_agent_binary 2>/dev/null
    function __gpy_resolve_agent_binary
        echo /usr/bin/true
    end

    functions -e __gpy_request 2>/dev/null
    function __gpy_request
        set -g __probe_ping_calls (math "$__probe_ping_calls + 1")
        echo pong
    end

    set -g __probe_ping_calls 0
    set -g GPY_AGENT_ENABLED 1
    set -g __gpy_agent_failure_count 0
    set -e __gpy_prompt_now
end

function test_expired_backoff_closes_the_breaker
    print_test_header "Unit Test: Circuit Breaker Closes After Its Backoff Elapses"
    install_probe_stubs

    # Backoff window ended a minute ago.
    set -g __gpy_agent_backoff_until (math (date +%s) - 60)

    __gpy_agent_available
    set -l rc $status

    if test $rc -ne 0
        print_test_result "Expired backoff closes the breaker" FAIL "expected availability 0, got $rc"
        return 1
    end
    if test $__probe_ping_calls -ne 1
        print_test_result "Expired backoff closes the breaker" FAIL "expected 1 ping, saw $__probe_ping_calls"
        return 1
    end
    print_test_result "Expired backoff closes the breaker" PASS
    return 0
end

function test_active_backoff_still_short_circuits
    print_test_header "Unit Test: Active Backoff Still Short-Circuits"
    install_probe_stubs

    # Backoff window is still open.
    set -g __gpy_agent_backoff_until (math (date +%s) + 600)

    __gpy_agent_available
    set -l rc $status

    if test $rc -ne 1
        print_test_result "Active backoff short-circuits" FAIL "expected availability 1, got $rc"
        return 1
    end
    if test $__probe_ping_calls -ne 0
        print_test_result "Active backoff short-circuits" FAIL "expected no ping, saw $__probe_ping_calls"
        return 1
    end
    print_test_result "Active backoff short-circuits" PASS
    return 0
end

function test_stale_prompt_timestamp_does_not_hold_the_breaker_open
    print_test_header "Unit Test: Stale Prompt Timestamp Does Not Hold The Breaker Open"
    install_probe_stubs

    # An idle shell -- exactly the one this recovery path exists for -- has not
    # rendered a prompt in a long time, so the per-render `__gpy_prompt_now`
    # memo is hours stale. The expiry check must not be fooled by it into
    # believing a long-elapsed backoff is still active.
    set -g __gpy_agent_backoff_until (math (date +%s) - 60)
    set -g __gpy_prompt_now (math (date +%s) - 3600)

    __gpy_agent_available
    set -l rc $status

    if test $rc -ne 0
        print_test_result "Stale prompt timestamp ignored for expiry" FAIL "expected availability 0, got $rc"
        return 1
    end
    print_test_result "Stale prompt timestamp ignored for expiry" PASS
    return 0
end

function test_breaker_opens_correctly_on_an_idle_shell
    print_test_header "Unit Test: Breaker Opens Correctly Despite A Stale Prompt Timestamp"
    install_probe_stubs

    # Ping always fails, as during an agent outage.
    functions -e __gpy_request 2>/dev/null
    function __gpy_request
        set -g __probe_ping_calls (math "$__probe_ping_calls + 1")
        echo ""
    end

    set -g __gpy_agent_backoff_until 0
    # Idle shell: its per-render timestamp memo is an hour stale. The backoff
    # deadline must be computed from real time, or it lands in the past and the
    # breaker never suppresses anything -- re-forking a ping on every call.
    set -g __gpy_prompt_now (math (date +%s) - 3600)

    for i in 1 2 3
        __gpy_agent_available >/dev/null 2>&1
    end
    set -l after_threshold $__probe_ping_calls

    __gpy_agent_available >/dev/null 2>&1

    if test $__probe_ping_calls -ne $after_threshold
        print_test_result "Breaker opens despite stale prompt timestamp" FAIL \
            "expected the 4th call to be suppressed, but it pinged (calls=$__probe_ping_calls)"
        return 1
    end
    print_test_result "Breaker opens despite stale prompt timestamp" PASS
    return 0
end

set -g __probe_failures 0

test_expired_backoff_closes_the_breaker; or set __probe_failures (math "$__probe_failures + 1")
test_active_backoff_still_short_circuits; or set __probe_failures (math "$__probe_failures + 1")
test_stale_prompt_timestamp_does_not_hold_the_breaker_open; or set __probe_failures (math "$__probe_failures + 1")
test_breaker_opens_correctly_on_an_idle_shell; or set __probe_failures (math "$__probe_failures + 1")

if test $__probe_failures -gt 0
    exit 1
end
exit 0
