#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Existing Client Re-registers After Agent Restart
#
# 1. An idle interactive Fish (fish/conf.d/gpy_init.fish and
#    fish/functions/fish_prompt.fish on a pseudo-terminal) stays registered
#    across an agent restart with no keystroke: the restarted agent writes
#    the shell's .reregister flag and rings SIGURG, the shell repaints, and
#    that `gpy prompt fish` re-registers. A working-tree edit made after the
#    restart then repaints the idle prompt with the dirty marker.
# 2. A shell whose first prompts all ran while no agent could start (the
#    `gpy-agent` beside its `gpy` exits at once) registers on its next prompt
#    once an agent is up.
#
# Skips (test_skip) without python3.

source (dirname (status -f))/../lib/test_helpers.fish

set -g __pty $__gpy_root/tests/lib/pty_session.py

function strip_escapes
    cat | string replace -ra '\e\][^\a\e]*(\a|\e\\\\)' '' | string replace -ra '\e\[[0-9;?>=<]*[A-Za-z]' '' | string replace -ra '\e[()=>][A-Za-z0-9]?' '' | string replace -a \r ''
end

function __session_has --argument-names dir needle offset
    tail -c +(math $offset + 1) $dir/transcript | strip_escapes | string collect | string match -q -- "*$needle*"
end

function registered_clients
    gpy-agent status 2>/dev/null | string match -rg 'registered_clients: (\d+)' | tail -1
end

function __clients_are --argument-names n
    test "$(registered_clients)" = $n
end

function test_idle_client_reregisters
    print_test_header "E2E Test: an idle shell re-registers after an agent restart"
    init_test_env
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '%s\n' '[ui]' 'theme = "text"' 'enabled_segments = ["directory", "git"]' >$XDG_CONFIG_HOME/gpy/config.toml

    set -l repo (create_test_repo)
    git -C $repo config core.fsmonitor false
    if not start_test_agent
        fail "Agent Start (initial)" "gpy-agent failed to start"
        return 1
    end
    pass "Agent Start (initial)"

    set -l dir $__gpy_test_tmp_dir/session
    env GPY_E2E_ROOT=$__gpy_root GPY_E2E_CWD=$repo \
        python3 $__pty start $dir -- fish --no-config -i -C '
        source $GPY_E2E_ROOT/fish/conf.d/gpy_init.fish
        source $GPY_E2E_ROOT/fish/functions/fish_prompt.fish
        cd $GPY_E2E_CWD
    '
    function __stop_session --inherit-variable dir
        python3 $__pty stop $dir >/dev/null 2>&1
    end
    test_track_cleanup __stop_session

    if not poll_until 10 __session_has $dir "❯" 0
        fail "first prompt rendered" "no prompt character within 10 s"
        return 1
    end
    if not poll_until 5 __clients_are 1
        fail "session registered with the agent" (registered_clients)
        return 1
    end
    pass "session registered with the agent"

    stop_test_agent >/dev/null
    if not start_test_agent
        fail "Agent Restart" "Unable to restart gpy-agent"
        return 1
    end
    pass "Agent Restart"

    # No keystroke from here on.
    if poll_until 10 __clients_are 1
        pass "the idle shell re-registered with the restarted agent"
    else
        fail "the idle shell re-registered with the restarted agent" "registered_clients: "(registered_clients)
        return 1
    end

    set -l off (python3 $__pty size $dir)
    echo dirty >>$repo/README.md
    if poll_until 10 __session_has $dir "✱1" $off
        pass "a later edit repaints the idle prompt"
    else
        fail "a later edit repaints the idle prompt" "no ✱1 render within 10 s"
        tail -c 1500 $dir/transcript | strip_escapes
    end
    __stop_session
    return 0
end

function test_failed_starts_client_recovers
    print_test_header "E2E Test: a shell after failed starts registers on its next prompt"
    init_test_env

    set -l target_dir $__gpy_root/gpy-agent/target
    test -n "$CARGO_TARGET_DIR"; and set target_dir $CARGO_TARGET_DIR
    set -l bin $__gpy_test_tmp_dir/bin
    mkdir -p $bin
    cp $target_dir/debug/gpy $bin/gpy
    # A crash-looping agent: every start exits at once without listening.
    printf '%s\n' '#!/bin/sh' 'exit 0' >$bin/gpy-agent
    chmod +x $bin/gpy-agent

    set -l repo (create_test_repo)
    set -l go $__gpy_test_tmp_dir/go
    set -l failed $__gpy_test_tmp_dir/failed
    set -l done $__gpy_test_tmp_dir/done
    env PATH=(string join : $bin $PATH) GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0 GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1 \
        fish --no-config -c '
        source $argv[1]/fish/functions/fish_prompt.fish
        cd $argv[2]
        for i in 1 2 3 4 5
            fish_prompt >/dev/null 2>&1
        end
        touch $argv[4]
        for i in (seq 150)
            test -e $argv[3]; and break
            sleep 0.1
        end
        fish_prompt >/dev/null 2>&1
        touch $argv[5]
        sleep 10' -- $__gpy_root $repo $go $failed $done &
    test_track_pid $last_pid

    if not poll_until 10 test -e $failed
        fail "prompts against an unavailable agent" "the shell did not finish its prompts"
        return 1
    end
    pass "prompts against an unavailable agent"

    if not start_test_agent
        fail "Agent Start (after failed starts)" "gpy-agent failed to start"
        return 1
    end
    check "no shell registered before the next prompt" 0 (registered_clients)
    touch $go
    poll_until 10 test -e $done
    if poll_until 5 __clients_are 1
        pass "one prompt after the agent came up registers the shell"
    else
        fail "one prompt after the agent came up registers the shell" "registered_clients: "(registered_clients)
    end
    return 0
end

if not command -q python3
    test_skip "python3 is required to drive a pseudo-terminal"
end

test_idle_client_reregisters
cleanup_test_files
test_failed_starts_client_recovers
cleanup_test_files
test_finish "existing client reregisters after restart"
