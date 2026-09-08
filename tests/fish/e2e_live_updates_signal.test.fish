#!/usr/bin/env fish
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::create_dir)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Live Update Signal Flow & Agent Restart Handling
# ============================================================================
#
# This test exercises the full git → watcher → agent → Fish shell pipeline.
# It verifies that:
#   1. The real agent delivers SIGUSR1 signals when repository changes occur.
#   2. The pipeline recovers after the agent is stopped and restarted.
#
# The test intentionally runs against the actual gpy-agent binary so we catch
# regressions in configuration loading, watcher setup, and signal dispatch.
#
# Expected runtime: ~10 seconds
#
# Limitations:
#   - The visible repaint itself is asserted on a real pseudo-terminal by
#     e2e_interactive_session.test.fish (#645)
#   - This test runs inside the current repository; it assumes `git add` works.

source (dirname (status -f))/../lib/test_helpers.fish

function print_banner
    print_test_header "E2E Test: Live Update Signal Flow"
end

function wait_for_signal --argument-names client_pid minimum_count timeout_seconds
    set -l deadline $timeout_seconds
    set -l attempts (math "ceil($deadline / 0.25)")
    for i in (seq 1 $attempts)
        set -l received (count_signals $client_pid)
        if test $received -ge $minimum_count
            return 0
        end
        sleep 0.25
    end
    return 1
end

function remove_client_from_cleanup --argument-names target_pid
    set -l remaining
    for pid in $__gpy_test_client_pids
        if test $pid -ne $target_pid
            set -a remaining $pid
        end
    end
    if set -q remaining[1]
        set __gpy_test_client_pids $remaining
    else
        set -e __gpy_test_client_pids
    end
end

function test_signal_flow
    print_banner
    init_test_env

    if start_test_agent
        print_test_result "Agent Start (initial)" "PASS"
    else
        print_test_result "Agent Start (initial)" "FAIL" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l client1 (register_test_client 12)
    if test -z "$client1"
        print_test_result "Register client (phase 1)" "FAIL" "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    print_test_result "Register client (phase 1)" "PASS"

    # Let registration-time signals settle, then capture a baseline. We assert an
    # INCREMENT caused specifically by the git change — a plain "≥1" check would be
    # satisfied by the registration signal alone and could not detect a regression
    # where change-driven signals stop firing.
    sleep 2
    set -l baseline1 (count_signals $client1)

    trigger_git_change (pwd) >/dev/null
    if wait_for_signal $client1 (math $baseline1 + 1) 6
        print_test_result "Watcher delivers SIGUSR1 on change" "PASS"
    else
        set -l observed (count_signals $client1)
        print_test_result "Watcher delivers SIGUSR1 on change" "FAIL" "Expected > $baseline1 signals, saw $observed"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files (pwd)
    kill -9 $client1 2>/dev/null
    remove_client_from_cleanup $client1

    stop_test_agent >/dev/null
    if start_test_agent
        print_test_result "Agent Restart" "PASS"
    else
        print_test_result "Agent Restart" "FAIL" "Unable to restart gpy-agent"
        cleanup_test_files
        return 1
    end

    set -l client2 (register_test_client 12)
    if test -z "$client2"
        print_test_result "Register client (phase 2)" "FAIL" "Failed to spawn test client after restart"
        cleanup_test_files
        return 1
    end
    print_test_result "Register client (phase 2)" "PASS"

    sleep 2
    set -l baseline2 (count_signals $client2)

    trigger_git_change (pwd) >/dev/null
    if wait_for_signal $client2 (math $baseline2 + 1) 6
        print_test_result "Watcher recovers after restart" "PASS"
    else
        set -l observed2 (count_signals $client2)
        print_test_result "Watcher recovers after restart" "FAIL" "Expected > $baseline2 signals after restart, saw $observed2"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files (pwd)
    kill -9 $client2 2>/dev/null
    remove_client_from_cleanup $client2

    cleanup_test_files
    return 0
end

if not test_signal_flow
    exit 1
end

exit 0
