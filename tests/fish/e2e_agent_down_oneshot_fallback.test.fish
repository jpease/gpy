#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: the git segment is right on a cold cache and with the agent down
#
# One Fish shell (fish/functions/fish_prompt.fish, i.e. `gpy prompt fish`)
# draws its prompts on request in a fresh repository:
#   1. agent up, cache cold (a never-queried repo): the very first prompt
#      already shows the git segment, not a blank waiting for a repaint;
#   2. agent stopped, working tree then edited: the next prompt shows the
#      edit (the dirty marker) instead of the stale clean state cached while
#      the agent was up (`gpy prompt` computes git itself when the socket is
#      gone), and it does not hang on the missing agent. Every cache entry is
#      stale (GPY_GIT_INSTANT_CACHE_TTL_SECONDS=0), so the test does not wait
#      out the TTL.
#
# Expected runtime: ~8 seconds

source (dirname (status -f))/../lib/test_helpers.fish

function __socket_absent
    not test -S "$GPY_AGENT_SOCKET_PATH"
end

# Ask the shell for prompt $n and wait (bounded) for it.
function draw --argument-names n
    touch $__gpy_test_tmp_dir/go-$n
    poll_until 10 test -e $__gpy_test_tmp_dir/done-$n
    string replace -ra '\e\[[0-9;]*m' '' <$__gpy_test_tmp_dir/out-$n | string collect
end

function test_agent_down
    print_test_header "E2E Test: git segment on a cold cache and with the agent down"
    init_test_env
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '%s\n' '[ui]' 'show_icons = false' 'enabled_segments = ["directory", "git"]' >$XDG_CONFIG_HOME/gpy/config.toml

    set -l repo (create_test_repo)
    set -gx GPY_GIT_INSTANT_CACHE_TTL_SECONDS 0
    set -l branch (git -C $repo branch --show-current)
    if test -z "$branch"
        fail "Create test repo" "Unable to create temporary git repo"
        return 1
    end

    if not start_test_agent
        fail "Agent Start" "gpy-agent failed to start"
        return 1
    end
    pass "Agent Start"

    # The shell draws prompt N once go-N exists and stops after prompt 2.
    fish --no-config -c '
        source $argv[1]/fish/functions/fish_prompt.fish
        cd $argv[2]
        for n in 1 2
            for i in (seq 200)
                test -e $argv[3]/go-$n; and break
                sleep 0.05
            end
            fish_prompt >$argv[3]/out-$n 2>&1
            touch $argv[3]/done-$n
        end' -- $__gpy_root $repo $__gpy_test_tmp_dir &
    test_track_pid $last_pid

    set -l first (draw 1)
    if string match -q "*$branch*" -- "$first"
        pass "the first prompt on a cold cache shows the git segment"
    else
        fail "the first prompt on a cold cache shows the git segment" "$first"
    end
    if string match -q '*✱*' -- "$first"
        fail "the first prompt shows a clean tree" "$first"
        return 1
    end

    stop_test_agent >/dev/null
    if not poll_until 4 __socket_absent
        fail "Agent socket removed after stop" "socket lingered; cannot exercise the agent-down path"
        return 1
    end
    echo dirty >>$repo/README.md

    set -l t0 (test_now_ns)
    set -l second (draw 2)
    set -l elapsed_ms (math --scale=0 "($(test_now_ns) - $t0) / 1000000")
    if string match -q "*$branch*✱1*" -- "$second"
        pass "with the agent down the prompt shows the edit"
    else
        fail "with the agent down the prompt shows the edit" "$second"
    end
    if test $elapsed_ms -lt 5000
        pass "the agent-down prompt does not hang ($elapsed_ms ms)"
    else
        fail "the agent-down prompt does not hang" "$elapsed_ms ms"
    end
    return 0
end

test_agent_down
cleanup_test_files
test_finish "agent down git segment"
