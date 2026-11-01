#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Existing Client Re-registers After Agent Restart
#
# This test covers the failure scenario where an already-open shell remains
# idle while the agent is stopped and restarted. The shell should recover and
# resume receiving SIGURG live-update signals without requiring a brand-new
# terminal window or an extra prompt render from the user.
#
# A second scenario covers the sibling
# registration-resilience gap: a shell whose initial start, registration retry
# and per-prompt restarts all failed while the agent was unavailable must still
# recover once the agent becomes available again, via the git segment's
# cold-miss/stale registration retry rather than the agent's .reregister
# doorbell.
#
# Note on "no cd / manual prompt render needed":
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

function test_existing_client_reregisters
    print_banner
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        fail "Create test repo" "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    if start_test_agent
        pass "Agent Start (initial)"
    else
        fail "Agent Start (initial)" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l client (register_test_client 18 $repo)
    if test -z "$client"
        fail "Register persistent client" "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    pass "Register persistent client"

    trigger_git_change $repo >/dev/null
    if wait_for_signal $client 1 6
        pass "Initial watcher signal"
    else
        set -l observed (count_signals $client)
        fail "Initial watcher signal" "Expected ≥1 signal, saw $observed"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files $repo

    stop_test_agent >/dev/null
    if start_test_agent
        pass "Agent Restart"
    else
        fail "Agent Restart" "Unable to restart gpy-agent"
        cleanup_test_files
        return 1
    end

    # Wait for the restarted agent to nudge (.reregister + SIGURG) and for the
    # already-open client to re-register. Polling the agent's client count
    # means we trigger the change only once the client is actually reattached.
    poll_until 4 __gpy_agent_has_registered_client

    # The restart nudge itself rings SIGURG, so count from the post-reattach
    # baseline: only a watcher repaint can raise it from here.
    set -l baseline_after_restart (count_signals $client)
    trigger_git_change $repo >/dev/null
    if wait_for_signal $client (math $baseline_after_restart + 1) 6
        pass "Existing client recovers"
    else
        set -l observed_after_restart (count_signals $client)
        fail "Existing client recovers" "Expected > $baseline_after_restart signals after restart, saw $observed_after_restart"
        cleanup_test_files
        return 1
    end

    cleanup_git_test_files $repo
    kill -9 $client 2>/dev/null
    remove_client_from_cleanup $client
    cleanup_test_files
    return 0
end

function test_failed_starts_client_recovers_via_git_segment
    print_test_header "E2E Test: Shell After Failed Starts Recovers via Git Segment"
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        fail "Create test repo" "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    set -l exhausted_marker $__gpy_test_tmp_dir/autostart_exhausted
    set -l go_marker $__gpy_test_tmp_dir/autostart_go
    set -l result_file $__gpy_test_tmp_dir/autostart_result
    rm -f $exhausted_marker $go_marker $result_file

    # Deliberately do NOT start the real agent yet: this client must run its
    # initial start, its registration retry and its checked prompts against a
    # genuinely unavailable agent first -- the failure scenario under test --
    # before the agent is ever brought up. The prompt handler is called
    # directly, since `fish -c` scripts never emit a real interactive
    # `fish_prompt` event.
    # Paths travel as $argv (never interpolated into the code string: they may
    # contain spaces or quotes); XDG_* and GPY_AGENT_SOCKET_PATH are already
    # exported by init_test_env and inherited.
    fish -c '
        set -l repo $argv[1]
        set -l __gpy_root $argv[2]
        set -l exhausted_marker $argv[3]
        set -l go_marker $argv[4]
        set -l result_file $argv[5]
        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
        # After the source: the theme export sets these.
        set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
        set -gx GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS 1

        # A crash-looping-binary stand-in: resolves and exits 0 immediately
        # without ever starting a listening agent, so every start fails
        # regardless of whether a real gpy-agent happens to be on PATH.
        functions -e __gpy_resolve_agent_binary 2>/dev/null
        function __gpy_resolve_agent_binary
            echo /usr/bin/true
        end

        cd $repo

        for i in 1 2 3 4 5
            __gpy_supervisor_on_prompt
        end

        if not functions -q __gpy_supervisor_on_prompt
            echo FAIL:handler-erased > $exhausted_marker
            exit 0
        end
        if test "$__gpy_supervisor_check_attempts" != 1
            echo FAIL:restart-attempts-$__gpy_supervisor_check_attempts > $exhausted_marker
            exit 0
        end
        if set -q __gpy_registered
            echo FAIL:registered-before-agent-started > $exhausted_marker
            exit 0
        end
        echo OK > $exhausted_marker

        # Later prompts would register through the check; pin the interval so
        # the single recovery under test is the git segment.
        set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 100000

        # Wait for the go-ahead of the outer test, given only once it has
        # confirmed (via the readiness poll of start_test_agent) that a real
        # agent is up and responding -- so the render below is the first and
        # only recovery attempt made against a genuinely available agent.
        set -l waited 0
        while not test -f $go_marker
            if test $waited -ge 150
                echo FAIL:go-marker-timeout > $result_file
                exit 0
            end
            sleep 0.1
            set waited (math "$waited + 1")
        end

        # A single cold-miss git-segment render -- the codepath that retries
        # registration -- must be enough to recover.
        segment_git_render >/dev/null 2>&1

        if set -q __gpy_registered
            echo PASS > $result_file
        else
            echo FAIL:not-registered-after-one-render > $result_file
        end
    ' -- $repo $__gpy_root $exhausted_marker $go_marker $result_file >/dev/null 2>&1 &

    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    if not poll_until 10 test -f $exhausted_marker
        fail "Failed starts against unavailable agent" "client did not finish its prompts within timeout"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    set -l exhausted_outcome (cat $exhausted_marker | string trim)
    if test "$exhausted_outcome" != OK
        fail "Failed starts against unavailable agent" "outcome=$exhausted_outcome"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end
    pass "Failed starts against unavailable agent"

    if start_test_agent
        pass "Agent Start (delayed, after failed starts)"
    else
        fail "Agent Start (delayed, after failed starts)" "gpy-agent failed to start"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    touch $go_marker

    if not poll_until 10 test -f $result_file
        fail "Client after failed starts re-registers within one render" "no result reported within timeout"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    set -l outcome (cat $result_file | string trim)
    kill -9 $client_pid 2>/dev/null
    remove_client_from_cleanup $client_pid

    if test "$outcome" = PASS
        pass "Client after failed starts re-registers within one render"
    else
        fail "Client after failed starts re-registers within one render" "outcome=$outcome"
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
        fail "Create test repo" "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    if start_test_agent
        pass "Agent Start"
    else
        fail "Agent Start" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    set -l result_file $__gpy_test_tmp_dir/renudge_result
    rm -f $result_file

    # A shell whose re-registration attempt fails during the restart nudge (in
    # production: the agent is not answering yet) must still re-register on the agent's
    # next nudge -- otherwise it is stranded unregistered for the rest of that
    # agent's lifetime, receiving no live updates at all. Each nudge is the
    # agent writing <pid>.reregister and ringing SIGURG; the doorbell consumes
    # the flag, so the retry relies on the agent writing it again.
    # Paths travel as $argv, never interpolated into the code string;
    # XDG_* and GPY_AGENT_SOCKET_PATH are inherited from init_test_env.
    fish -c '
        set -l repo $argv[1]
        set -l __gpy_root $argv[2]
        set -l result_file $argv[3]

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1

        cd $repo

        set -l reregister_flag (__gpy_shell_registry_file).reregister
        mkdir -p (dirname $reregister_flag)
        touch $reregister_flag

        # Simulate the real trigger: the agent does not answer yet when the
        # restart nudge lands.
        function __gpy_agent_available; return 1; end

        __gpy_doorbell_handler

        if set -q __gpy_registered
            echo FAIL:registered-despite-unavailable-agent > $result_file
            exit 0
        end
        if test -e $reregister_flag
            echo FAIL:flag-not-consumed > $result_file
            exit 0
        end

        # The agent has been up the whole time. The re-nudge of the agent
        # writes the flag again and must now succeed.
        source $__gpy_root/fish/core/util.fish

        touch $reregister_flag
        __gpy_doorbell_handler

        if set -q __gpy_registered
            echo PASS > $result_file
        else
            echo FAIL:no-retry-on-repeat-nudge > $result_file
        end
    ' -- $repo $__gpy_root $result_file >/dev/null 2>&1

    if not test -f $result_file
        fail "Failed nudge retries on the next nudge" "client reported no result"
        cleanup_test_files
        return 1
    end

    set -l outcome (cat $result_file | string trim)
    if test "$outcome" = PASS
        pass "Failed nudge retries on the next nudge"
    else
        fail "Failed nudge retries on the next nudge" "outcome=$outcome"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

# Predicate for poll_until: the restart nudge reached the client.
function __restart_nudge_landed --argument-names flag pid registrations_before
    test -e $flag; and return 0
    set -l now (grep -c "Registering client: PID=$pid," $__gpy_test_debug_log)
    test "$now" -gt "$registrations_before"
end

# Truncated directory display whose prefix is $symbol.
function __write_truncated_directory_config --argument-names symbol
    printf '[ui.directory]\ndisplay = "truncated"\ntruncation_length = 1\ntruncation_symbol = "%s"\n' $symbol >$XDG_CONFIG_HOME/gpy/config.toml
end

function test_restart_drops_memoized_directory_render
    print_test_header "E2E Test: Re-registration Drops Memoized Agent Renders"
    init_test_env

    set -l repo (create_test_repo)
    if test -z "$repo"
        fail "Create test repo" "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    mkdir -p $XDG_CONFIG_HOME/gpy
    __write_truncated_directory_config OLD/
    if start_test_agent
        pass "Agent Start"
    else
        fail "Agent Start" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end
    # Like an interactive shell, the client must source the export cache:
    # without it, re-registration reloads the theme and clears every memo
    # whether or not it drops them itself.
    if not poll_until 5 test -f $XDG_CACHE_HOME/gpy/theme-export.fish
        fail "Theme export cache written" "no $XDG_CACHE_HOME/gpy/theme-export.fish"
        cleanup_test_files
        return 1
    end

    set -l before_file $__gpy_test_tmp_dir/memo_before
    set -l go_marker $__gpy_test_tmp_dir/memo_go
    set -l after_file $__gpy_test_tmp_dir/memo_after
    rm -f $before_file $go_marker $after_file

    # The directory memo is keyed on theme, PWD, position and prev_bg, never
    # on config. The shell memoizes a render, the agent restarts with another
    # truncation symbol (not in the theme export, so no `.reload` doorbell),
    # and re-registration must drop the memo so the next render is the new
    # agent's. Paths travel as $argv; XDG_* and GPY_AGENT_SOCKET_PATH are
    # inherited from init_test_env.
    fish -c '
        set -l repo $argv[1]
        set -l __gpy_root $argv[2]
        set -l before_file $argv[3]
        set -l go_marker $argv[4]
        set -l after_file $argv[5]

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
        cd $repo
        __gpy_register_with_agent >/dev/null 2>&1
        __gpy_shell_registry_file >$before_file.registry
        set -q __gpy_theme_export_mtime[1]; and touch $before_file.sourced
        # fish_prompt resets prev_bg each render; the memo key includes it.
        set -g __gpy_last_segment_bg black
        segment_directory_render "" true >$before_file

        set -l waited 0
        while not test -f $go_marker
            if test $waited -ge 150
                echo FAIL:go-marker-timeout >$after_file
                exit 0
            end
            sleep 0.1
            set waited (math "$waited + 1")
        end

        # The SIGURG handler may already have run on the nudge; if not, the
        # flag is still there and this consumes it.
        __gpy_doorbell_handler
        set -g __gpy_last_segment_bg black
        segment_directory_render "" true >$after_file
    ' -- $repo $__gpy_root $before_file $go_marker $after_file >/dev/null 2>&1 &
    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    if not poll_until 10 test -s $before_file
        fail "Memoized the first directory render" "no render within 10 s"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end
    pass "Memoized the first directory render"
    if not test -e $before_file.sourced
        fail "Client sourced the theme export cache" "the fixture would reload the theme on re-registration"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    set -l registry_file (cat $before_file.registry)
    set -l registrations_before (grep -c "Registering client: PID=$client_pid," $__gpy_test_debug_log)
    stop_test_agent >/dev/null
    __write_truncated_directory_config TRUNC/
    if not start_test_agent
        fail "Agent Restart" "Unable to restart gpy-agent"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end

    # The restart nudge has landed: the flag is waiting, or the client's
    # SIGURG handler already consumed it and re-registered.
    if not poll_until 5 __restart_nudge_landed $registry_file.reregister $client_pid $registrations_before
        fail "Restart nudge" "no .reregister flag or re-registration for $client_pid"
        kill -9 $client_pid 2>/dev/null
        cleanup_test_files
        return 1
    end
    touch $go_marker

    poll_until 10 test -s $after_file
    kill -9 $client_pid 2>/dev/null
    remove_client_from_cleanup $client_pid

    set -l before (cat $before_file 2>/dev/null)
    set -l after (cat $after_file 2>/dev/null)
    if not string match -q '*OLD/*' -- "$before"
        fail "Re-registration drops the directory memo" "the first render does not show the first config: '$before'"
        cleanup_test_files
        return 1
    else if string match -q '*TRUNC/*' -- "$after"
        pass "Re-registration drops the directory memo"
    else
        fail "Re-registration drops the directory memo" "render after restart: '$after' (before: '$before')"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_existing_client_reregisters
    exit 1
end

if not test_failed_starts_client_recovers_via_git_segment
    exit 1
end

if not test_failed_restart_registration_retries_on_next_nudge
    exit 1
end

test_restart_drops_memoized_directory_render; or exit 1
test_finish "existing client reregisters after restart"
