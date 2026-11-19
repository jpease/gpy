#!/usr/bin/env fish
# tests/fish/first_prompt_agent_start.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The shell holds no protocol or version knowledge: the first prompt runs
# `gpy-agent start` exactly once even when the agent already answers (`start`
# keeps or replaces the agent itself), a later prompt never starts it again
# while it answers, and an unreachable agent is still started, capped by the
# restart attempt limit. Registration is attempted on every prompt until it
# succeeds, also with the supervisor disabled and with no gpy-agent binary.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/flags.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"
source "$repo_root/tests/lib/shell_render_only.fish"

print_test_header "The first prompt starts the agent once"

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
set -gx PATH $tmp $PATH
set -g calls $tmp/agent.calls
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$calls'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent

# Registration stand-in: succeeds once the socket exists, and counts attempts.
set -g attempts 0
function __gpy_register_with_agent
    set -q __gpy_registered; and return 0
    set -g attempts (math $attempts + 1)
    test -S $GPY_AGENT_SOCKET_PATH; or return 1
    set -g __gpy_registered 1
end
function __gpy_sync_workspace
end

function __starts
    test -f $calls; or begin
        echo 0
        return
    end
    count (string match -r '^start$' <$calls)
end

function __starts_at_least --argument-names minimum
    test (__starts) -ge $minimum
end

function __fresh_shell
    rm -f $tmp/agent.sock $calls
    set -g attempts 0
    set -e __gpy_registered
    set -g __gpy_supervisor_check_attempts 0
    set -e __gpy_supervisor_initialized
    set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
    set -e GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS
    set -e GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS
end

# --- (a)+(b): a healthy agent is started once, registers now, not started again ---

__fresh_shell
test_stub_listener $tmp/agent.sock reply --lifetime 30 --reply '{"status":"ok"}'
__gpy_supervisor_on_prompt 2>/dev/null
poll_until 5 __starts_at_least 1; or fail "a healthy agent did not make the first prompt run start"
check "the first prompt registers at once with a healthy agent" 1 (set -q __gpy_registered; and echo 1; or echo 0)

# A second prompt past the rate limit probes a healthy agent: no start, and a
# registered shell makes no further attempt.
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
__gpy_supervisor_on_prompt 2>/dev/null
__gpy_supervisor_on_prompt 2>/dev/null
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a healthy agent gets exactly one start per shell" 1 (__starts)
check "a registered shell makes one registration attempt in all" 1 $attempts
test_stop_pids $test_stub_pid

# --- (c): an unreachable agent is started; later prompts retry registration ---

__fresh_shell
__gpy_supervisor_on_prompt 2>/dev/null
poll_until 5 __starts_at_least 1; or fail "an unreachable agent did not make the first prompt run start"
check "the first prompt does not register with an unreachable agent" 0 (set -q __gpy_registered; and echo 1; or echo 0)

__gpy_supervisor_on_prompt 2>/dev/null
__gpy_supervisor_on_prompt 2>/dev/null
sleep 0.5
check "every prompt retries registration while unregistered" 3 $attempts
check "the next prompts do not start the agent again" 1 (__starts)

# The agent comes up: the next prompt registers, whatever the supervisor says.
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
test_stub_listener $tmp/agent.sock reply --lifetime 30 --reply '{"status":"ok"}'
__gpy_supervisor_on_prompt 2>/dev/null
check "a prompt after the agent came up registers with the supervisor disabled" 1 (set -q __gpy_registered; and echo 1; or echo 0)
test_stop_pids $test_stub_pid

# --- registration retries need neither the supervisor nor the binary ---

__fresh_shell
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
functions -c __gpy_resolve_agent_binary __gpy_resolve_agent_binary_real
function __gpy_resolve_agent_binary
    return 1
end
for i in 1 2 3
    __gpy_supervisor_on_prompt 2>/dev/null
end
check "three prompts with no supervisor and no binary make three registration attempts" 3 $attempts
check "no binary means no start" 0 (__starts)
functions -e __gpy_resolve_agent_binary
functions -c __gpy_resolve_agent_binary_real __gpy_resolve_agent_binary

# --- attempt cap: later prompts restart a dead agent, at most max-attempts times ---

__fresh_shell
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
set -gx GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS 2
__gpy_supervisor_on_prompt 2>/dev/null # first prompt: the startup start
for i in 1 2 3 4 5 6
    __gpy_supervisor_on_prompt 2>/dev/null
end
# One startup start plus two capped restarts.
poll_until 5 __starts_at_least 3; or fail "an unreachable agent was not restarted up to the attempt cap"
sleep 0.5
check "an unreachable agent gets the startup start plus the capped restarts" 3 (__starts)

test_finish "The first prompt starts the agent once"
