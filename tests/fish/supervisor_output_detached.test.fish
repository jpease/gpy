#!/usr/bin/env fish
# tests/fish/supervisor_output_detached.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Fish agent supervisor must not keep the spawning shell's stdout/stderr:
# with GPY_VERBOSE=1 and an unhealthy agent, nothing it logs may reach the
# spawning shell's output.

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -l tmp (mktemp -d)
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_CACHE_HOME $tmp/cache
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/fish $XDG_CACHE_HOME
ln -s $repo_root/fish $XDG_CONFIG_HOME/fish/gpy
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid
test_track_dir $tmp
test_track_pidfile $pidfile

# A stub agent binary that records each restart attempt: the supervisor
# running through check intervals is what the assertion below needs, and the
# call log is how the test observes it.
set -g calls $tmp/agent-calls
printf '%s\n' '#!/bin/sh' "echo call >> '$calls'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent

env GPY_VERBOSE=1 GPY_AGENT_BINARY_PATH=$tmp/gpy-agent \
    GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS=0 \
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=1 \
    fish --no-config -c '
        source $argv[1]/fish/core/constants.fish
        source $argv[1]/fish/core/util.fish
        source $argv[1]/fish/core/ipc.fish
        __gpy_agent_supervisor_start' -- $repo_root >$tmp/out 2>&1

if not poll_until 3 test -s $pidfile
    fail "Supervisor never wrote $pidfile"
    exit 1
end

# Two restart attempts means a full check interval of supervisor logging has
# happened; only then is "nothing reached the spawning shell" meaningful.
function __two_restart_attempts
    test (count (cat $calls 2>/dev/null)) -ge 2
end
if not poll_until 10 __two_restart_attempts
    fail "Supervisor made "(count (cat $calls 2>/dev/null))" restart attempt(s) in 10s, expected at least 2"
    exit 1
end

if test -s $tmp/out
    fail "Supervisor output reached the spawning shell:"
    cat $tmp/out
    exit 1
end

test_finish "Supervisor output is detached from the spawning shell"
