#!/usr/bin/env fish
# __gpy_agent_restart on a machine without `lsof` (#850).
#
# When `gpy-agent stop` leaves a live process on the socket, restart finds the
# holder with `lsof -t` and kills it. Minimal Debian/Ubuntu/WSL images ship no
# lsof, so the stale agent was never killed -- and the old code then deleted its
# socket file anyway, orphaning the process, before starting a second agent.
#
# Now: lsof first, `fuser` (psmisc, present on most of those images) second.
# With neither, the live agent is left alone -- its socket file is NOT removed,
# a warning names the missing tool, and `gpy-agent start` (which reclaims a
# stale socket and evicts a wedged agent on its own) is still run.
#
# A python3 listener plays the lingering agent; a stub `gpy-agent` whose `stop`
# does nothing stands in for an agent that ignores the shutdown request, and
# logs `start`. lsof and fuser are stubs that print the listener's PID.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Agent restart without lsof (#850)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a lingering agent"
end

set -g tmp (mktemp -d)
set -g real_path $PATH
set -g failures 0
set -g listener_pid

function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        print_test_result "$label" FAIL "$detail"
        set -g failures (math $failures + 1)
    end
end

function finish
    test -n "$listener_pid"; and kill -9 $listener_pid 2>/dev/null
    set -gx PATH $real_path
    rm -rf $tmp
end

# The tools restart itself needs, and nothing else: no lsof, no fuser, no client.
mkdir -p $tmp/base
for tool in rm sleep kill
    ln -sf (command -v $tool) $tmp/base/$tool
end

# Stub agent: `stop` is ignored (a wedged agent), `start` is logged.
set -g start_log $tmp/start.log
printf '%s\n' '#!/bin/sh' 'echo "$1" >> "$GPY_TEST_AGENT_LOG"' 'exit 0' >$tmp/base/gpy-agent
chmod +x $tmp/base/gpy-agent

# start_lingering_agent: a listener bound to $tmp/agent.sock for up to 30s.
function start_lingering_agent
    rm -f $tmp/agent.sock
    python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1]); s.listen(4)
time.sleep(30)
' $tmp/agent.sock &
    set -g listener_pid $last_pid
    for i in (seq 1 30)
        test -S $tmp/agent.sock; and return 0
        sleep 0.1
    end
    finish
    test_skip "could not create test Unix socket listener"
end

function alive
    kill -0 $listener_pid 2>/dev/null
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
sleep 0.2
check "no lsof/fuser: the lingering agent is not killed" (alive; and echo 1; or echo 0) "the listener died"
check "no lsof/fuser: its socket file is not deleted" (test -S $tmp/agent.sock; and echo 1; or echo 0) "the socket file was removed under a live agent"
check "no lsof/fuser: the warning names the missing tool" (string match -q '*lsof*' -- (string collect $out); and echo 1; or echo 0) "output: $out"
check "no lsof/fuser: gpy-agent start still runs" (grep -qx start $start_log; and echo 1; or echo 0) "start log: "(cat $start_log)
kill -9 $listener_pid 2>/dev/null

# --- lsof present: the holder is killed and the socket cleaned up -------------
start_lingering_agent
mkdir -p $tmp/withlsof
printf '%s\n' '#!/bin/sh' "echo $listener_pid" >$tmp/withlsof/lsof
chmod +x $tmp/withlsof/lsof
run_restart $tmp/withlsof >/dev/null
sleep 0.3
check "lsof: the lingering agent is killed" (alive; and echo 0; or echo 1) "the listener survived"
check "lsof: its stale socket file is cleaned up" (test -S $tmp/agent.sock; and echo 0; or echo 1) "the socket file remains"
check "lsof: gpy-agent start runs" (grep -qx start $start_log; and echo 1; or echo 0) "start log: "(cat $start_log)

# --- only fuser present (the minimal-image case) ------------------------------
start_lingering_agent
mkdir -p $tmp/withfuser
# psmisc: the path goes to stderr, the PIDs to stdout.
printf '%s\n' '#!/bin/sh' "echo \"\$1:\" >&2" "echo \" $listener_pid\"" >$tmp/withfuser/fuser
chmod +x $tmp/withfuser/fuser
run_restart $tmp/withfuser >/dev/null
sleep 0.3
check "fuser fallback: the lingering agent is killed" (alive; and echo 0; or echo 1) "the listener survived"
check "fuser fallback: its stale socket file is cleaned up" (test -S $tmp/agent.sock; and echo 0; or echo 1) "the socket file remains"

finish
if test $failures -gt 0
    print_test_footer "Agent restart without lsof" FAIL
    exit 1
end
print_test_footer "Agent restart without lsof" PASS
