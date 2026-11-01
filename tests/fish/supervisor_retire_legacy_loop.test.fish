#!/usr/bin/env fish
# tests/fish/supervisor_retire_legacy_loop.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A detached supervisor loop left by an earlier version is terminated at the
# first prompt and its pidfile removed; a pidfile naming a foreign process is
# removed without signalling that process.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"

print_test_header "The first prompt retires a legacy supervisor loop"

if not command -q python3
    test_skip "python3 not installed, cannot start a fake loop"
end

set -g tmp (mktemp -d)
test_track_dir $tmp
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx GPY_AGENT_ENABLED 1
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid
mkdir -p (path dirname $pidfile)

# The agent side is not under test.
function __gpy_ensure_agent_started; return 0; end
function __gpy_register_with_agent; return 0; end

# Gone, or a zombie nobody has reaped yet.
function __pid_gone --argument-names pid
    kill -0 $pid 2>/dev/null; or return 0
    ps -o stat= -p $pid 2>/dev/null | string match -q 'Z*'
end

# A process whose command line carries the loop's function name.
python3 -c 'import time; time.sleep(60)' __gpy_agent_supervisor_loop &
set -l fake_loop $last_pid
test_track_pid $fake_loop
# A bystander whose command line does not.
python3 -c 'import time; time.sleep(60)' &
set -l foreign $last_pid
test_track_pid $foreign

# ps may need a moment to show the new command line.
function __sees_marker --argument-names pid
    __gpy_is_supervisor_pid $pid
end
poll_until 5 __sees_marker $fake_loop; or fail "the fake loop's command line never showed up in ps"

echo $fake_loop >$pidfile
set -e __gpy_supervisor_initialized
__gpy_supervisor_on_prompt
poll_until 5 __pid_gone $fake_loop
__pid_gone $fake_loop; or fail "the recorded loop was not terminated"
test -e $pidfile; and fail "the pidfile of the loop was left behind"
pass "the recorded loop is terminated and its pidfile removed"

echo $foreign >$pidfile
set -e __gpy_supervisor_initialized
__gpy_supervisor_on_prompt
# Absence of a signal cannot be polled for: give a wrongly sent TERM time to land.
sleep 0.5
kill -0 $foreign 2>/dev/null; or fail "a foreign PID in supervisor.pid was signalled"
test -e $pidfile; and fail "the pidfile naming a foreign process was left behind"
pass "a foreign PID is not signalled"

test_finish "The first prompt retires a legacy supervisor loop"
