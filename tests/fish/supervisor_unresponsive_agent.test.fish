#!/usr/bin/env fish
# The per-prompt check:
#   - __gpy_agent_is_healthy tells "unreachable" (1: no socket, nothing
#     listening) from "connected but silent" (2), against real sockets;
#   - one failed check runs `gpy-agent start` (recorded by a stub binary), and
#     a healthy check resets the attempt count.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"

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

# --- per-prompt check over scripted health results ---

set -gx XDG_CACHE_HOME $tmp/cache
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
set -g start_log $tmp/starts.log
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >> '$start_log'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent
set -gx GPY_AGENT_BINARY_PATH $tmp/gpy-agent

function __gpy_agent_is_healthy
    set -g check_no (math $check_no + 1)
    return $health_seq[$check_no]
end

function __start_count
    test -f $start_log; or begin
        echo 0
        return
    end
    count (string match -r '^start$' <$start_log)
end

function __starts_reach --argument-names want
    test (__start_count) -ge $want
end

# One failed check, one start: a silent agent is replaced on the first check,
# `gpy-agent start` itself deciding whether it is wedged.
set -g health_seq 2
set -g check_no 0
set -g __gpy_supervisor_last_check_time 0
set -g __gpy_supervisor_check_attempts 0
__gpy_supervisor_check
set rc $status
check "a failed check reports the agent down" 1 $rc
poll_until 5 __starts_reach 1; or fail "a failed check did not run gpy-agent start"
check "one failed check runs start once" 1 (__start_count)

# A healthy check answers 0 and resets the attempt count.
set -g health_seq 0
set -g check_no 0
set -g __gpy_supervisor_last_check_time 0
__gpy_supervisor_check
set rc $status
check "a healthy check returns 0" 0 $rc
check "a healthy check resets the attempt count" 0 $__gpy_supervisor_check_attempts

test_finish "Supervisor tolerates a briefly unresponsive agent"
