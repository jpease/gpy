#!/usr/bin/env fish
# tests/fish/agent_disabled_no_supervisor.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With the agent disabled in config.toml (exported GPY_AGENT_ENABLED=0), the
# first prompt must not spawn a supervisor or start the agent, and a running
# supervisor loop must exit instead of restarting the agent (#699).

set -l repo_root (path resolve (status dirname)/../..)

set -l tmp (mktemp -d)
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_CACHE_HOME $tmp/cache
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/fish $XDG_CACHE_HOME/gpy
ln -s $repo_root/fish $XDG_CONFIG_HOME/fish/gpy
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid
set -g stub_log $tmp/agent-calls.log

printf '%s\n' 'set -gx GPY_AGENT_ENABLED "0"' 'set -gx GPY_AGENT_SUPERVISOR_ENABLED "1"' >$XDG_CACHE_HOME/gpy/theme-export.fish

printf '%s\n' '#!/bin/sh' "echo \"\$*\" >> '$stub_log'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent
set -gx GPY_AGENT_BINARY_PATH $tmp/gpy-agent

function cleanup --inherit-variable tmp
    if test -f $pidfile
        set -l pid (cat $pidfile 2>/dev/null)
        test -n "$pid"; and kill $pid 2>/dev/null
    end
    rm -rf $tmp
end

function fail
    echo "❌ $argv"
    cleanup
    exit 1
end

# Case 1: the first prompt spawns nothing.
source $repo_root/fish/core/init.fish
test "$GPY_AGENT_ENABLED" = 0; or fail "theme export did not set GPY_AGENT_ENABLED=0 (got '$GPY_AGENT_ENABLED')"
emit fish_prompt
sleep 1

test -e $pidfile; and fail "supervisor.pid was created with GPY_AGENT_ENABLED=0"
if test -f $stub_log; and string match -q -r '^start' <$stub_log
    fail "gpy-agent start was called with GPY_AGENT_ENABLED=0"
end

# Returns 0 once $pid has exited, 1 if it is still alive after $seconds.
function poll_exit --argument-names seconds pid
    for i in (seq (math "$seconds * 10"))
        kill -0 $pid 2>/dev/null; or return 0
        sleep 0.1
    end
    return 1
end

# Case 2: a supervisor loop exits at once instead of restarting the agent.
env GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=1 \
    fish --no-config -c '
        source $argv[1]/fish/core/constants.fish
        source $argv[1]/fish/core/util.fish
        source $argv[1]/fish/core/ipc.fish
        function __gpy_agent_is_healthy; return 1; end
        function __gpy_agent_restart; echo RESTART; return 1; end
        __gpy_agent_supervisor_loop' -- $repo_root >$tmp/loop.out 2>/dev/null &
set -l loop_pid $last_pid
if not poll_exit 5 $loop_pid
    kill $loop_pid 2>/dev/null
    fail "supervisor loop kept running with GPY_AGENT_ENABLED=0"
end
string match -q RESTART <$tmp/loop.out; and fail "supervisor loop restarted the agent with GPY_AGENT_ENABLED=0"

cleanup
echo "✅ Agent-disabled shells spawn no supervisor and the loop exits"
