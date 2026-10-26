#!/usr/bin/env fish
# A restart kills the running agent, so the supervisor must not mistake a
# brief stall for a dead agent:
#   - __gpy_agent_is_healthy tells "unreachable" (1: no socket, nothing
#     listening) from "connected but silent" (2), against real sockets;
#   - the loop restarts an unreachable agent on the first check, but a silent
#     one only after two failed checks in a row, and a healthy check clears
#     that streak.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Supervisor tolerates a briefly unresponsive agent"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a silent agent"
end
if not __gpy_ipc_client_available
    test_skip "neither socat nor an nc with -U is installed"
end

set -g tmp (mktemp -d)
test_track_dir $tmp
# --- __gpy_agent_is_healthy against real sockets ---

set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock

__gpy_agent_is_healthy
set -l rc $status
check "no socket is unreachable (1)" 1 $rc

# A socket file nobody listens on: what a crashed agent leaves behind.
test_stub_listener $tmp/agent.sock bind-only
__gpy_agent_is_healthy
set rc $status
check "a socket nobody listens on is unreachable (1)" 1 $rc
rm -f $tmp/agent.sock

# A listener that accepts and never answers: a stalled agent.
test_stub_listener $tmp/agent.sock silent --lifetime 15
__gpy_agent_is_healthy
set rc $status
test_stop_pids $test_stub_pid
check "a connected agent that never answers is unresponsive (2)" 2 $rc

# --- loop decisions over a scripted sequence of health results ---

set -gx XDG_CACHE_HOME $tmp/cache
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 1
for fn in __gpy_log_warn __gpy_log_info __gpy_log_error
    function $fn
    end
end
function __gpy_resolve_agent_binary
    echo stub
end
function __gpy_agent_is_healthy
    set -g check_no (math $check_no + 1)
    return $health_seq[$check_no]
end
function __gpy_agent_restart
    set -ga restarted_at $check_no
    return 0
end
# Ends the loop once the scripted checks are used up; no real delay.
function sleep
    test $check_no -ge (count $health_seq); and set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
    return 0
end

# run_loop RESULT...: the check numbers at which the loop restarted the agent.
function run_loop
    set -g health_seq $argv
    set -g check_no 0
    set -g restarted_at
    set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
    __gpy_agent_supervisor_loop ""
    echo $restarted_at
end

set -l got (run_loop 2 0 2 0)
check "one silent check at a time never restarts" 1 (test -z "$got"; and echo 1; or echo 0)

set got (run_loop 0 2 2)
check "two silent checks in a row restart on the second" 3 "$got"

set got (run_loop 0 1)
check "an unreachable agent restarts on the first failed check" 2 "$got"

test_finish "Supervisor tolerates a briefly unresponsive agent"
