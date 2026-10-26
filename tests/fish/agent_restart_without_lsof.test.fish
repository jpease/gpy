#!/usr/bin/env fish
# __gpy_agent_restart on a machine without `lsof`.
#
# When `gpy-agent stop` leaves a live process on the socket, restart finds the
# holder with `lsof -t` and kills it. Minimal Debian/Ubuntu/WSL images ship no
# lsof, so restart falls back to `fuser` (psmisc, present on most of those
# images). With neither, the live agent is left alone -- its socket file is NOT
# removed (that would orphan the process before a second agent starts), a
# warning names the missing tool, and `gpy-agent start` (which reclaims a
# stale socket and evicts a wedged agent on its own) is still run.
#
# A stub listener plays the lingering agent; a stub `gpy-agent` whose `stop`
# does nothing stands in for an agent that ignores the shutdown request, and
# logs `start`. lsof and fuser are stubs that print the listener's PID.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Agent restart without lsof"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a lingering agent"
end

set -g tmp (mktemp -d)
set -g real_path $PATH

function finish
    set -gx PATH $real_path
    rm -rf $tmp
end

# Runs on every exit path, and first, so PATH is whole again for the guard's own
# tools even when the test died inside run_restart.
test_track_cleanup finish

# The tools restart itself needs, and nothing else: no lsof, no fuser, no client.
mkdir -p $tmp/base
for tool in rm sleep kill
    ln -sf (command -v $tool) $tmp/base/$tool
end

# Stub agent: `stop` is ignored (a wedged agent), `start` is logged.
set -g start_log $tmp/start.log
printf '%s\n' '#!/bin/sh' 'echo "$1" >> "$GPY_TEST_AGENT_LOG"' 'exit 0' >$tmp/base/gpy-agent
chmod +x $tmp/base/gpy-agent

# start_lingering_agent: a silent listener bound to $tmp/agent.sock for up to 30s.
function start_lingering_agent
    rm -f $tmp/agent.sock
    test_stub_listener $tmp/agent.sock silent --lifetime 30
end

function alive
    kill -0 $test_stub_pid 2>/dev/null
end

# The listener is dead and its socket file removed.
function holder_cleaned_up
    not alive; and not test -S $tmp/agent.sock
end

# run_restart LABEL EXTRA_DIR: restart with PATH = base (+ EXTRA_DIR); echoes
# the restart's stderr so the warning can be checked.
function run_restart --argument-names extra
    : >$start_log
    set -gx GPY_TEST_AGENT_LOG $start_log
    # __gpy_log_warn only speaks under GPY_DEBUG, so a prompt never prints it.
    set -lx GPY_DEBUG 1
    set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock
    set -g GPY_SOCKET_WAIT_MAX_ATTEMPTS 1
    set -e GPY_AGENT_BINARY_PATH
    set -gx PATH $tmp/base
    test -n "$extra"; and set -gx PATH $extra $tmp/base
    __gpy_agent_restart 2>&1
    set -gx PATH $real_path
end

# --- Neither lsof nor fuser: leave the live agent alone -----------------------
start_lingering_agent
set -l out (run_restart)
# Give a (wrong) kill a moment to land before asserting the listener survived.
sleep 0.2
check "no lsof/fuser: the lingering agent is not killed" 1 (alive; and echo 1; or echo 0)
check "no lsof/fuser: its socket file is not deleted" 1 (test -S $tmp/agent.sock; and echo 1; or echo 0)
check "no lsof/fuser: the warning names the missing tool" 1 (string match -q '*lsof*' -- (string collect $out); and echo 1; or echo 0)
check "no lsof/fuser: gpy-agent start still runs" 1 (grep -qx start $start_log; and echo 1; or echo 0)
test_stop_pids $test_stub_pid

# --- lsof present: the holder is killed and the socket cleaned up -------------
start_lingering_agent
mkdir -p $tmp/withlsof
printf '%s\n' '#!/bin/sh' "echo $test_stub_pid" >$tmp/withlsof/lsof
chmod +x $tmp/withlsof/lsof
run_restart $tmp/withlsof >/dev/null
poll_until 3 holder_cleaned_up
check "lsof: the lingering agent is killed" 1 (alive; and echo 0; or echo 1)
check "lsof: its stale socket file is cleaned up" 1 (test -S $tmp/agent.sock; and echo 0; or echo 1)
check "lsof: gpy-agent start runs" 1 (grep -qx start $start_log; and echo 1; or echo 0)

# --- only fuser present (the minimal-image case) ------------------------------
start_lingering_agent
mkdir -p $tmp/withfuser
# psmisc: the path goes to stderr, the PIDs to stdout.
printf '%s\n' '#!/bin/sh' "echo \"\$1:\" >&2" "echo \" $test_stub_pid\"" >$tmp/withfuser/fuser
chmod +x $tmp/withfuser/fuser
run_restart $tmp/withfuser >/dev/null
poll_until 3 holder_cleaned_up
check "fuser fallback: the lingering agent is killed" 1 (alive; and echo 0; or echo 1)
check "fuser fallback: its stale socket file is cleaned up" 1 (test -S $tmp/agent.sock; and echo 0; or echo 1)

finish
test_finish "Agent restart without lsof"
