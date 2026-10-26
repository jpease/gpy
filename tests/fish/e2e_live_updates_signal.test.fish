#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Live Update Signal Flow & Agent Restart Handling
#
# This test exercises the full git → watcher → agent → Fish shell pipeline.
# It verifies that:
#   1. The real agent rings SIGURG when repository changes occur.
#   2. The pipeline recovers after the agent is stopped and restarted.
#
# The test intentionally runs against the actual gpy-agent binary so we catch
# regressions in configuration loading, watcher setup, and signal dispatch.
#
# Expected runtime: ~10 seconds
#
# Limitations:
#   - The visible repaint itself is asserted on a real pseudo-terminal by
#     e2e_interactive_session.test.fish
#   - This test runs inside the current repository; it assumes `git add` works.

source (dirname (status -f))/../lib/test_helpers.fish

function print_banner
    print_test_header "E2E Test: Live Update Signal Flow"
end

# Make git changes until the client's signal count exceeds the baseline. The
# agent exposes no "watch ready" signal, so a change made before the watch is
# armed is retried rather than waited out.
function __gpy_change_rang --argument-names client_pid baseline
    echo >>(pwd)/.gpy-test-$fish_pid
    git -C (pwd) add (pwd)/.gpy-test-$fish_pid >/dev/null 2>&1
    test (count_signals $client_pid) -gt $baseline
end

function ring_after_change --argument-names client_pid baseline
    poll_until 8 __gpy_change_rang $client_pid $baseline
end

function test_signal_flow
    print_banner
    init_test_env

    if start_test_agent
        pass "Agent Start (initial)"
    else
        fail "Agent Start (initial)" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l client1 (register_test_client 12)
    if test -z "$client1"
        fail "Register client (phase 1)" "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    pass "Register client (phase 1)"

    # Wait for the registration signal, then capture a baseline. We assert an
    # INCREMENT caused specifically by the git change: a plain "at least 1" check
    # would be satisfied by the registration signal alone.
    wait_for_signal $client1 1 5
    set -l baseline1 (count_signals $client1)

    if ring_after_change $client1 $baseline1
        pass "Watcher rings SIGURG on change"
    else
        set -l observed (count_signals $client1)
        fail "Watcher rings SIGURG on change" "Expected > $baseline1 signals, saw $observed"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files (pwd)
    kill -9 $client1 2>/dev/null
    remove_client_from_cleanup $client1

    stop_test_agent >/dev/null
    if start_test_agent
        pass "Agent Restart"
    else
        fail "Agent Restart" "Unable to restart gpy-agent"
        cleanup_test_files
        return 1
    end

    set -l client2 (register_test_client 12)
    if test -z "$client2"
        fail "Register client (phase 2)" "Failed to spawn test client after restart"
        cleanup_test_files
        return 1
    end
    pass "Register client (phase 2)"

    wait_for_signal $client2 1 5
    set -l baseline2 (count_signals $client2)

    if ring_after_change $client2 $baseline2
        pass "Watcher recovers after restart"
    else
        set -l observed2 (count_signals $client2)
        fail "Watcher recovers after restart" "Expected > $baseline2 signals after restart, saw $observed2"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files (pwd)
    kill -9 $client2 2>/dev/null
    remove_client_from_cleanup $client2

    cleanup_test_files
    return 0
end

test_signal_flow; or exit 1
test_finish "live updates signal"
