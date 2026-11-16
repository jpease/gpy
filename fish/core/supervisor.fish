# SPDX-License-Identifier: GPL-3.0-or-later

# 0: healthy. 1: unreachable (no socket, nothing listening). 2: connected but
# no valid reply, which a briefly stalled agent also produces. Runs on the
# prompt, so it waits no longer than the prompt's IPC budget.
function __gpy_agent_is_healthy --description 'Check the agent: 0 healthy, 1 unreachable, 2 unresponsive'
    set -l budget (__gpy_ipc_timeout_ms)
    set -l response (__gpy_ipc_send '{"op":"ping"}' $budget)
    test $status -eq 1; and return 1

    if string match -q '*"status":"ok"*' -- "$response"
        return 0
    end
    return 2
end

# `start` is idempotent and replaces a wedged or older-protocol agent itself.
function __gpy_start_agent_detached --description 'Run gpy-agent start in the background without waiting'
    set -l agent_binary (__gpy_resolve_agent_binary)
    or return 1
    "$agent_binary" start >/dev/null 2>&1 &
    disown $last_pid 2>/dev/null
    return 0
end

set -g __gpy_supervisor_last_check_time 0
set -g __gpy_supervisor_check_attempts 0

# Per-prompt restart of a dead agent, rate limited and attempt capped.
# Returns 0 only when this call found the agent answering.
function __gpy_supervisor_check --description 'Prompt-time agent probe; starts the agent when it is down'
    __gpy_agent_enabled; and __gpy_supervisor_enabled; or return 1

    # Rate-limit before any IPC so a healthy agent costs no round-trips. Read
    # at call time so a config reload applies.
    set -l rate_limit (__gpy_supervisor_check_interval)
    set -l now (__gpy_now_seconds)
    string match -qr '^[0-9]+$' -- "$now"; or return 1
    test (math "$now - $__gpy_supervisor_last_check_time") -ge $rate_limit; or return 1
    set -g __gpy_supervisor_last_check_time $now

    if __gpy_agent_is_healthy
        set -g __gpy_supervisor_check_attempts 0
        return 0
    end

    set -l max_attempts (__gpy_supervisor_max_restarts)
    test $__gpy_supervisor_check_attempts -lt $max_attempts; or return 1
    set -g __gpy_supervisor_check_attempts (math "$__gpy_supervisor_check_attempts + 1")
    __gpy_start_agent_detached
    return 1
end

# The first prompt starts the agent in the background, without probing it:
# `start` is idempotent and itself keeps a healthy agent or replaces one on
# another protocol or version (left running across an upgrade). Later prompts
# run the rate-limited restart check. Registration is retried on every prompt
# until it succeeds, whatever the supervisor setting and whether or not the
# binary resolves; with no socket file that costs no fork. The workspace sync
# runs here too, once per prompt and only when $PWD changed.
function __gpy_supervisor_on_prompt --on-event fish_prompt --description 'Start the agent on the first prompt, then check it on later prompts'
    __gpy_agent_enabled; or return 0

    if not set -q __gpy_supervisor_initialized
        set -g __gpy_supervisor_initialized 1
        # The startup start counts as the first check.
        set -g __gpy_supervisor_last_check_time (__gpy_now_seconds)
        __gpy_start_agent_detached
    else
        __gpy_supervisor_check
    end

    if not set -q __gpy_registered
        __gpy_register_with_agent
    end
    __gpy_sync_workspace
    return 0
end

function __gpy_unregister_on_exit --on-event fish_exit
    # Unregistering untracks; a shell that lost its registration is still
    # tracked for the agent's re-nudge, so untrack it here.
    if set -q __gpy_registered
        __gpy_unregister_from_agent
    else
        __gpy_untrack_shell_for_agent_recovery
    end
end
