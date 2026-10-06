#!/usr/bin/env fish
# tests/fish/supervisor_singleton.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# At most one Fish agent supervisor runs per runtime root, and supervisor.pid
# names it. A stale pidfile naming a live non-supervisor PID does not block a
# new supervisor (#703).

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -g tmp (mktemp -d)
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_CACHE_HOME $tmp/cache
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/fish $XDG_CACHE_HOME
ln -s $repo_root/fish $XDG_CONFIG_HOME/fish/gpy
set -gx GPY_AGENT_BINARY_PATH /usr/bin/true
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 3600
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid
set -g sleep_pid

# Supervisors are matched by this sandbox's pidfile path (an argv of the
# child), never by process name (#484).
function supervisors
    pgrep -f $pidfile
end

function cleanup
    for pid in (supervisors)
        kill $pid 2>/dev/null
    end
    test -n "$sleep_pid"; and kill $sleep_pid 2>/dev/null
    rm -rf $tmp
end

function fail
    echo "❌ $argv"
    cleanup
    exit 1
end

function start_supervisor --inherit-variable repo_root
    fish --no-config -c '
        source $argv[1]/fish/core/constants.fish
        source $argv[1]/fish/core/util.fish
        source $argv[1]/fish/core/ipc.fish
        __gpy_agent_supervisor_start' -- $repo_root
end

function pidfile_written
    test -s $pidfile
end

function supervisor_running
    test (count (supervisors)) -eq 1
end

function pidfile_names_supervisor
    test "$(cat $pidfile 2>/dev/null)" = "$(supervisors)"
end

# Case 1: three concurrent starts leave exactly one supervisor.
start_supervisor &
start_supervisor &
start_supervisor &
wait
poll_until 3 pidfile_written; or fail "no supervisor wrote $pidfile"
sleep 1

set -l running (supervisors)
test (count $running) -eq 1; or fail "expected 1 supervisor, found "(count $running)": $running"
test "$running" = (cat $pidfile); or fail "supervisor.pid ("(cat $pidfile)") does not name the running supervisor ($running)"

for pid in $running
    kill $pid
end

# Case 2: a pidfile naming a live non-supervisor PID does not block a start.
sleep 30 &
set sleep_pid $last_pid
mkdir -p (path dirname $pidfile)
echo $sleep_pid >$pidfile
start_supervisor
poll_until 3 supervisor_running; or fail "a stale pidfile naming a live non-supervisor PID blocked the supervisor"
poll_until 3 pidfile_names_supervisor; or fail "supervisor did not claim the stale pidfile"

cleanup
echo "✅ One supervisor per runtime root"
