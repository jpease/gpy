#!/usr/bin/env fish
# Test for the SIGURG doorbell
#
# The agent rings a shell with SIGURG; the flag files that say why are read
# by the next `gpy prompt fish`. Fish's job is only to repaint: every ring
# bumps __gpy_repaint_trigger, whose variable handler runs
# `commandline -f force-repaint`. A handler the user installed for SIGURG
# keeps running beside it. The real signal is delivered with `kill -URG` to a
# fresh interactive Fish that loaded fish/conf.d/gpy_init.fish.
#
# The visual repaint itself needs a terminal; see
# tests/fish/e2e_interactive_session.test.fish (a real pty session).
#
# Run with: fish tests/fish/doorbell_signal.test.fish

set -l repo_root (path resolve (dirname (status -f))/../..)
source $repo_root/tests/lib/test_helpers.fish

set -l out (env GPY_AGENT_ENABLED=0 fish --no-config -i -c '
    function user_urg --on-signal SIGURG
        set -g user_rings (math (set -q user_rings; and echo $user_rings; or echo 0) + 1)
    end
    source $argv[1]/fish/conf.d/gpy_init.fish
    source $argv[1]/fish/functions/fish_prompt.fish
    # Signal handlers run at the next safe point: wait (bounded) for each.
    function ring
        set -l before "$__gpy_repaint_trigger"
        kill -URG $fish_pid
        for i in (seq 20)
            test "$__gpy_repaint_trigger" != "$before"; and return
            sleep 0.1
        end
    end
    echo "before=$__gpy_repaint_trigger"
    ring
    echo first=$__gpy_repaint_trigger
    ring
    ring
    echo third=$__gpy_repaint_trigger
    echo user=$user_rings
    echo alive' -- $repo_root </dev/null 2>&1 | string collect)

check "no ring yet: no trigger" 1 (string match -qr '(?m)^before=$' -- "$out"; and echo 1)
check "a ring bumps the repaint trigger" 1 (string match -q '*first=1*' -- "$out"; and echo 1)
check "every ring bumps it" 1 (string match -q '*third=3*' -- "$out"; and echo 1)
check "the user's own SIGURG handler still runs" 1 (string match -q '*user=3*' -- "$out"; and echo 1)
check "the shell survives the rings" 1 (string match -q '*alive*' -- "$out"; and echo 1)

# Without GPY loaded, SIGURG is ignored: an early ring (an `exec fish`
# before conf.d ran) does not kill the shell.
set out (fish --no-config -c 'kill -URG $fish_pid; sleep 0.2; echo alive' </dev/null 2>&1 | string collect)
check "a shell without GPY survives a ring" alive "$out"

test_finish "doorbell signal"
