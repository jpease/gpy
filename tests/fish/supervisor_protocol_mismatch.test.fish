#!/usr/bin/env fish
# tests/fish/supervisor_protocol_mismatch.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A reachable agent that reports another protocol version counts as not
# reachable: the first prompt runs `gpy-agent start` once (which replaces the
# older agent), and a matching agent is left alone.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"

print_test_header "Protocol mismatch starts the agent on the first prompt"

if not command -q python3
    test_skip "python3 not installed, cannot simulate an agent"
end
if not __gpy_ipc_client_available
    test_skip "neither socat nor an nc with -U is installed"
end

set -g tmp (mktemp -d)
test_track_dir $tmp
set -gx XDG_CACHE_HOME $tmp/cache
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
set -gx GPY_AGENT_BINARY_PATH $tmp/gpy-agent
set -g calls $tmp/agent.calls
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$calls'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent

function __gpy_register_with_agent; return 0; end

function __starts
    test -f $calls; or begin
        echo 0
        return
    end
    count (string match -r '^start$' <$calls)
end

function __first_prompt_with_protocol --argument-names protocol
    rm -f $tmp/agent.sock $calls
    set -e __gpy_supervisor_initialized
    set -e __gpy_register_retry_pending
    test_stub_listener $tmp/agent.sock reply --lifetime 30 \
        --reply '{"status":"ok","version":"0.0.0","protocol_version":'$protocol'}'
    __gpy_supervisor_on_prompt 2>/dev/null
end

function __start_recorded
    test (__starts) -ge 1
end

__first_prompt_with_protocol 1
poll_until 5 __start_recorded; or fail "a protocol 1 agent did not make the first prompt run start"
check "a mismatched agent is started over exactly once" 1 (__starts)
test_stop_pids $test_stub_pid

__first_prompt_with_protocol 2
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a matching agent is left alone" 0 (__starts)
test_stop_pids $test_stub_pid

test_finish "Protocol mismatch starts the agent on the first prompt"
