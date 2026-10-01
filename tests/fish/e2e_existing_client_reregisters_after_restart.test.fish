#!/usr/bin/env fish
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::create_dir)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Existing Client Re-registers After Agent Restart
# ============================================================================
#
# This test covers the failure scenario where an already-open shell remains
# idle while the agent is stopped and restarted. The shell should recover and
# resume receiving SIGURG live-update signals without requiring a brand-new
# terminal window or an extra prompt render from the user.
#
# gpy#419 extends this file with a second scenario covering the sibling
# registration-resilience gap: a shell whose per-prompt autostart attempts
# were exhausted while the agent was unavailable (which deletes the
# self-repeating `__gpy_start_supervisor_on_prompt` hook) must still recover
# once the agent becomes available again, via the git segment's cold-miss/
# stale registration retry rather than the agent's .reregister doorbell.
#
# Note on "no cd / manual prompt render needed" (acceptance criterion 1):
# `test_existing_client_reregisters` below already proves this for the
# doorbell path -- the persistent client (`register_test_client`)
# registers once, then only sleeps waiting for signals; it never calls `cd`
# or re-renders a prompt between the initial registration and the recovery
# assertion. Recovery there comes entirely from the agent's restart nudge
# (.reregister flag + SIGURG).

source (dirname (status -f))/../lib/test_helpers.fish

function print_banner
    print_test_header "E2E Test: Existing Client Re-registers After Restart"
end

function wait_for_signal --argument-names client_pid minimum_count timeout_seconds
    set -l attempts (math "ceil($timeout_seconds / 0.25)")
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

function test_existing_client_reregisters
    print_banner
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    if start_test_agent
        print_test_result "Agent Start (initial)" PASS
    else
        print_test_result "Agent Start (initial)" FAIL "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l client (register_test_client 18 $repo)
    if test -z "$client"
        print_test_result "Register persistent client" FAIL "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    print_test_result "Register persistent client" PASS

    trigger_git_change $repo >/dev/null
    if wait_for_signal $client 1 6
        print_test_result "Initial watcher signal" PASS
    else
        set -l observed (count_signals $client)
        print_test_result "Initial watcher signal" FAIL "Expected ≥1 signal, saw $observed"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files $repo

    stop_test_agent >/dev/null
    if start_test_agent
        print_test_result "Agent Restart" PASS
    else
        print_test_result "Agent Restart" FAIL "Unable to restart gpy-agent"
        cleanup_test_files
        return 1
    end

    # Wait for the restarted agent to nudge (.reregister + SIGURG) and for the
    # already-open client to re-register (was: fixed `sleep 2`). Polling the
    # agent's client count means we trigger the change only once the client is
    # actually reattached, which is both faster and less racy.
    poll_until 4 __gpy_agent_has_registered_client

    # The restart nudge itself rings SIGURG, so count from the post-reattach
    # baseline: only a watcher repaint can raise it from here.
    set -l baseline_after_restart (count_signals $client)
    trigger_git_change $repo >/dev/null
    if wait_for_signal $client (math $baseline_after_restart + 1) 6
        print_test_result "Existing client recovers" PASS
    else
        set -l observed_after_restart (count_signals $client)
        print_test_result "Existing client recovers" FAIL "Expected > $baseline_after_restart signals after restart, saw $observed_after_restart"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files $repo
    kill -9 $client 2>/dev/null
    remove_client_from_cleanup $client
    cleanup_test_files
    return 0
end

function test_autostart_exhausted_client_recovers_via_git_segment
    print_test_header "E2E Test: Autostart-Exhausted Shell Recovers via Git Segment"
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    set -l exhausted_marker $__gpy_test_tmp_dir/autostart_exhausted
    set -l go_marker $__gpy_test_tmp_dir/autostart_go
    set -l result_file $__gpy_test_tmp_dir/autostart_result
    rm -f $exhausted_marker $go_marker $result_file

    # Deliberately do NOT start the real agent yet: this client must exhaust
    # its per-prompt autostart attempts against a genuinely unavailable agent
    # first -- exactly #419's failure scenario -- before the agent is ever
    # brought up. The event handlers are simulated by calling them directly
    # (same technique as tests/fish/prompt_autostart_backoff.test.fish),
    # since `fish -c` scripts never emit a real interactive `fish_prompt`
    # event.
    fish -c "
        set -gx XDG_CONFIG_HOME $XDG_CONFIG_HOME
        set -gx XDG_CACHE_HOME $XDG_CACHE_HOME
        set -gx GPY_AGENT_SOCKET_PATH $GPY_AGENT_SOCKET_PATH
        set -gx GPY_AGENT_AUTOSTART_MAX_ATTEMPTS 1
        set -gx GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS 0
        set -gx GPY_AGENT_START_DELAY_MS 0

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1

        # A crash-looping-binary stand-in: resolves and exits 0 immediately
        # without ever starting a listening agent, so __gpy_agent_available
        # keeps failing and exhaustion is genuine regardless of whether a
        # real gpy-agent happens to be on PATH.
        functions -e __gpy_resolve_agent_binary 2>/dev/null
        function __gpy_resolve_agent_binary
            echo /usr/bin/true
        end

        # Avoid leaking a detached background supervisor process for the
        # life of the test run; it only restarts the agent *process* and
        # plays no part in the fish-side registration recovery under test.
        functions -e __gpy_agent_supervisor_start 2>/dev/null
        function __gpy_agent_supervisor_start
            return 0
        end

        cd $repo

        for i in 1 2 3 4 5
            if functions -q __gpy_start_supervisor_on_prompt
                __gpy_start_supervisor_on_prompt
            end
        end

        if functions -q __gpy_start_supervisor_on_prompt
            echo 'FAIL:not-exhausted' > $exhausted_marker
            exit 0
        end
        if set -q __gpy_registered
            echo 'FAIL:registered-before-agent-started' > $exhausted_marker
            exit 0
        end
        echo OK > $exhausted_marker

        # The exhaustion loop above hammers __gpy_agent_available and will
        # have opened its circuit breaker (util.fish, a real pre-existing
        # protection this issue doesn't touch and isn't in scope to change).
        # Reset it here to simulate that breaker's normal backoff window
        # having already elapsed by the time the agent comes back -- without
        # this the test would need a real 60s sleep to exercise the
        # registration-retry path #419 actually adds, rather than just
        # re-observing the pre-existing, orthogonal circuit breaker.
        set -g __gpy_agent_backoff_until 0
        set -g __gpy_agent_failure_count 0

        # Wait for the outer test's go-ahead, given only once it has
        # confirmed (via start_test_agent's own readiness poll) that a real
        # agent is up and responding -- so the render below is the first and
        # only recovery attempt made against a genuinely available agent.
        set -l waited 0
        while not test -f $go_marker
            if test \$waited -ge 150
                echo 'FAIL:go-marker-timeout' > $result_file
                exit 0
            end
            sleep 0.1
            set waited (math \"\$waited + 1\")
        end

        # A single cold-miss git-segment render -- the exact codepath #419
        # adds registration retry to -- must be enough to recover.
        segment_git_render >/dev/null 2>&1

        if set -q __gpy_registered
            echo PASS > $result_file
        else
            echo 'FAIL:not-registered-after-one-render' > $result_file
        end
    " >/dev/null 2>&1 &

    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    if not poll_until 10 test -f $exhausted_marker
        print_test_result "Autostart exhausts against unavailable agent" FAIL "client did not reach exhaustion within timeout"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    set -l exhausted_outcome (cat $exhausted_marker | string trim)
    if test "$exhausted_outcome" != OK
        print_test_result "Autostart exhausts against unavailable agent" FAIL "outcome=$exhausted_outcome"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end
    print_test_result "Autostart exhausts against unavailable agent" PASS

    if start_test_agent
        print_test_result "Agent Start (delayed, after exhaustion)" PASS
    else
        print_test_result "Agent Start (delayed, after exhaustion)" FAIL "gpy-agent failed to start"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    touch $go_marker

    if not poll_until 10 test -f $result_file
        print_test_result "Exhausted client re-registers within one render" FAIL "no result reported within timeout"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    set -l outcome (cat $result_file | string trim)
    kill -9 $client_pid 2>/dev/null
    remove_client_from_cleanup $client_pid

    if test "$outcome" = PASS
        print_test_result "Exhausted client re-registers within one render" PASS
    else
        print_test_result "Exhausted client re-registers within one render" FAIL "outcome=$outcome"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

function test_failed_restart_registration_retries_on_next_nudge
    print_test_header "E2E Test: Failed Restart Re-registration Retries On Next Nudge"
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    if start_test_agent
        print_test_result "Agent Start" PASS
    else
        print_test_result "Agent Start" FAIL "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l result_file $__gpy_test_tmp_dir/renudge_result
    rm -f $result_file

    # A shell whose re-registration attempt fails during the restart nudge (in
    # production: its circuit breaker is still backing off from the pings it
    # made while the agent was down) must still re-register on the agent's
    # next nudge -- otherwise it is stranded unregistered for the rest of that
    # agent's lifetime, receiving no live updates at all. Each nudge is the
    # agent writing <pid>.reregister and ringing SIGURG; the doorbell consumes
    # the flag, so the retry relies on the agent writing it again.
    fish -c "
        set -gx XDG_CONFIG_HOME $XDG_CONFIG_HOME
        set -gx XDG_CACHE_HOME $XDG_CACHE_HOME
        set -gx GPY_AGENT_SOCKET_PATH $GPY_AGENT_SOCKET_PATH

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1

        cd $repo

        set -l reregister_flag (__gpy_shell_registry_file).reregister
        mkdir -p (dirname \$reregister_flag)
        touch \$reregister_flag

        # Simulate the real trigger: the circuit breaker opened while the agent
        # was down, so this shell refuses to talk to the agent for the length of
        # its backoff -- which the restart nudge lands inside.
        set -g __gpy_agent_backoff_until (math (date +%s) + 600)

        __gpy_doorbell_handler

        if set -q __gpy_registered
            echo 'FAIL:registered-despite-open-breaker' > $result_file
            exit 0
        end
        if test -e \$reregister_flag
            echo 'FAIL:flag-not-consumed' > $result_file
            exit 0
        end

        # Backoff window elapses; the agent has been up the whole time. The
        # agent's re-nudge writes the flag again and must now succeed.
        set -g __gpy_agent_backoff_until 0
        set -g __gpy_agent_failure_count 0

        touch \$reregister_flag
        __gpy_doorbell_handler

        if set -q __gpy_registered
            echo PASS > $result_file
        else
            echo 'FAIL:no-retry-on-repeat-nudge' > $result_file
        end
    " >/dev/null 2>&1

    if not test -f $result_file
        print_test_result "Failed nudge retries on the next nudge" FAIL "client reported no result"
        cleanup_test_files
        return 1
    end

    set -l outcome (cat $result_file | string trim)
    if test "$outcome" = PASS
        print_test_result "Failed nudge retries on the next nudge" PASS
    else
        print_test_result "Failed nudge retries on the next nudge" FAIL "outcome=$outcome"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_existing_client_reregisters
    exit 1
end

if not test_autostart_exhausted_client_recovers_via_git_segment
    exit 1
end

if not test_failed_restart_registration_retries_on_next_nudge
    exit 1
end

exit 0
