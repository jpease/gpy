#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Theme Refresh After an Offline Config Edit
#
# A config.toml change made while the agent is down is never seen by the
# agent's file watcher, so no `.reload` is produced for it. When the agent
# next starts it re-warms the theme-export cache for the new config; every
# shell must then pick up the new theme instead of keeping the variables it
# sourced from the stale cache:
#
#   A. A new shell that sourced the stale cache, then autostarts the agent on
#      its first prompt (registration-time freshness check, fish side).
#   B. A shell open across agent stop -> config edit -> agent start, without
#      rendering a prompt.
#   C. Like B, but with the shell-side freshness check stubbed out, so only
#      the agent's own `.reload` doorbell for tracked shells can deliver the
#      new theme (agent side).

source (dirname (status -f))/../lib/test_helpers.fish

# The theme-export cache holds the export for theme `want` (every export
# carries its theme's name).
function __cache_theme_is --argument-names want
    set -l cache $XDG_CACHE_HOME/gpy/theme-export.fish
    test -f $cache; or return 1
    string match -q -- "set -g __gpy_theme_name \"$want\"" <$cache
end

function __write_config_theme --argument-names theme
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '[ui]\ntheme = "%s"\n' $theme >$XDG_CONFIG_HOME/gpy/config.toml
end

# Isolated env shared by every scenario: the debug agent (never an installed
# one on PATH), and the shell-tracking dir under the isolated XDG_CACHE_HOME
# rather than a real XDG_RUNTIME_DIR.
function __setup_offline_edit_env
    init_test_env
    set -e XDG_RUNTIME_DIR
    set -gx PATH $__gpy_root/gpy-agent/target/debug $PATH
    set -gx GPY_AGENT_BINARY_PATH $__gpy_root/gpy-agent/target/debug/gpy-agent
    mkdir -p $__gpy_test_tmp_dir/plain
end

# Start the agent on `theme = "default"` and wait for its cache warm-up, so the
# cache a shell sources holds the `default` export.
function __warm_default_cache
    __write_config_theme default
    start_test_agent; or return 1
    poll_until 5 __cache_theme_is default
end

function test_new_shell_after_offline_edit
    print_test_header "E2E Test: New Shell Picks Up an Offline Config Edit"
    __setup_offline_edit_env

    if not __warm_default_cache
        fail "Cache warmed with default theme" "agent did not write the default export"
        cleanup_test_files
        return 1
    end
    stop_test_agent
    pass "Cache warmed with default theme"

    # The shell compares the cache's mtime (1 s resolution) with the one it
    # sourced; the offline edit must land in a later second than the stale
    # write, as it does in real use. Backdate the stale write rather than
    # waiting out the second.
    test_backdate $XDG_CACHE_HOME/gpy/theme-export.fish 5
    __write_config_theme text

    set -l result_file $__gpy_test_tmp_dir/new_shell_result
    # Paths travel as $argv, never interpolated into the code string. The
    # first prompt starts the agent in the background and does not wait, so a
    # second prompt, sent once the agent answers, registers (the autostarted
    # agent is stopped by cleanup_test_files through the test socket).
    fish -c '
        set -l __gpy_root $argv[1]
        set -l result_file $argv[2]
        set -l plain_dir $argv[3]

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
        cd $plain_dir

        if test "$__gpy_theme_name" != default
            echo "FAIL:stale-cache-not-sourced theme=$__gpy_theme_name" >$result_file
            exit 0
        end

        emit fish_prompt

        # The start is detached: wait for the agent, then let the next prompt
        # retry the registration.
        for i in (seq 100)
            __gpy_ipc_send \'{"op":"ping"}\' 500 >/dev/null 2>&1; and break
            sleep 0.1
        end
        emit fish_prompt

        if not set -q __gpy_registered
            echo FAIL:not-registered-after-autostart >$result_file
            exit 0
        end

        for i in (seq 50)
            test "$__gpy_theme_name" = text; and break
            sleep 0.1
        end

        if test "$__gpy_theme_name" = text
            echo PASS >$result_file
        else
            echo "FAIL:theme=$__gpy_theme_name" >$result_file
        end
    ' -- $__gpy_root $result_file $__gpy_test_tmp_dir/plain >/dev/null 2>&1

    set -l outcome (cat $result_file 2>/dev/null | string trim)
    if test "$outcome" = PASS
        pass "New shell shows the new theme after autostart"
    else
        fail "New shell shows the new theme after autostart" "outcome=$outcome"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

# Shared body of scenarios B and C. With `agent_only` the shell-side freshness
# check is stubbed out, so the theme can only arrive through the agent's
# `.reload` doorbell.
function __run_existing_shell_scenario --argument-names label agent_only
    __setup_offline_edit_env

    if not __warm_default_cache
        fail "Cache warmed with default theme" "agent did not write the default export"
        cleanup_test_files
        return 1
    end
    pass "Cache warmed with default theme"

    set -l ready_file $__gpy_test_tmp_dir/existing_ready
    set -l state_file $__gpy_test_tmp_dir/existing_state
    set -l result_file $__gpy_test_tmp_dir/existing_result

    fish -c '
        set -l __gpy_root $argv[1]
        set -l ready_file $argv[2]
        set -l state_file $argv[3]
        set -l result_file $argv[4]
        set -l plain_dir $argv[5]
        set -l agent_only $argv[6]

        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
        cd $plain_dir
        if test "$agent_only" = 1
            function __gpy_reload_if_theme_export_changed
                return 0
            end
        end

        if not __gpy_register_with_agent >/dev/null 2>&1
            echo FAIL:not-registered >$ready_file
            exit 0
        end
        if test "$__gpy_theme_name" != default
            echo "FAIL:theme=$__gpy_theme_name" >$ready_file
            exit 0
        end
        echo OK >$ready_file

        # Idle like a shell at its prompt: no prompt render, only doorbells.
        for i in (seq 300)
            echo $__gpy_theme_name >$state_file
            if test "$__gpy_theme_name" = text
                echo PASS >$result_file
                exit 0
            end
            sleep 0.1
        end
    ' -- $__gpy_root $ready_file $state_file $result_file $__gpy_test_tmp_dir/plain $agent_only >/dev/null 2>&1 &
    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    poll_until 10 test -f $ready_file
    set -l ready (cat $ready_file 2>/dev/null | string trim)
    if test "$ready" != OK
        fail "Existing shell registered on default theme" "outcome=$ready"
        cleanup_test_files
        return 1
    end
    pass "Existing shell registered on default theme"

    stop_test_agent
    __write_config_theme text
    if not start_test_agent
        fail "Agent restart" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    if poll_until 10 test -f $result_file
        pass "$label"
    else
        set -l observed (cat $state_file 2>/dev/null | string trim)
        fail "$label" "theme stayed '$observed'"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

function test_existing_shell_after_offline_edit
    print_test_header "E2E Test: Existing Shell Picks Up an Offline Config Edit"
    __run_existing_shell_scenario "Existing shell shows the new theme after restart" 0
end

function test_existing_shell_reload_doorbell_after_offline_edit
    print_test_header "E2E Test: Agent Rings .reload For an Offline Config Edit"
    __run_existing_shell_scenario "Shell reloads via the agent's .reload doorbell alone" 1
end

test_new_shell_after_offline_edit
test_existing_shell_after_offline_edit
test_existing_shell_reload_doorbell_after_offline_edit
test_finish "theme refresh after offline edit"
