# SPDX-License-Identifier: GPL-3.0-or-later

# Live supervisor, not merely a live (possibly recycled) PID.
function __gpy_is_supervisor_pid --argument-names pid
    string match -qr '^\d+$' -- "$pid"; or return 1
    ps -o command= -p $pid 2>/dev/null | string match -q '*__gpy_agent_supervisor_loop*'
end

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

# A loop left running by an earlier version exits only when its pidfile
# changes, so stop it once.
function __gpy_retire_legacy_supervisor --description 'Stop a detached supervisor loop left by an earlier version'
    set -l pidfile (__gpy_runtime_root)/supervisor.pid
    test -f "$pidfile"; or return 0
    set -l pid (cat "$pidfile" 2>/dev/null)
    if __gpy_is_supervisor_pid "$pid"
        kill -TERM $pid 2>/dev/null
    end
    rm -f "$pidfile"
    return 0
end

function __gpy_warn_deprecated_supervisor_knobs --description 'Warn once about removed supervisor tunables still exported'
    for name in GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS GPY_SUPERVISOR_LONG_BACKOFF_SECONDS GPY_AGENT_AUTOSTART_MAX_ATTEMPTS GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS
        if set -q $name
            echo "gpy: $name is deprecated and has no effect; use [agent.supervisor] in config.toml" >&2
        end
    end
end

# Returns 0 when the agent is reachable and speaks this protocol; otherwise it
# starts the agent in the background and returns 1.
function __gpy_ensure_agent_started --description 'Start the agent in the background unless it is reachable'
    __gpy_agent_enabled; or return 1
    __gpy_resolve_agent_binary >/dev/null; or return 1

    # A reachable agent on another protocol counts as not reachable.
    if __gpy_agent_is_healthy; and __gpy_check_protocol_version
        return 0
    end

    __gpy_start_agent_detached
    set -g __gpy_register_retry_pending 1
    return 1
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

function __gpy_supervisor_on_prompt --on-event fish_prompt --description 'Start the agent on the first prompt, then check it on later prompts'
    __gpy_agent_enabled; or return 0

    if not set -q __gpy_supervisor_initialized
        set -g __gpy_supervisor_initialized 1
        __gpy_warn_deprecated_supervisor_knobs
        __gpy_retire_legacy_supervisor
        # The probe just made counts as the first check.
        set -g __gpy_supervisor_last_check_time (__gpy_now_seconds)
        if __gpy_ensure_agent_started
            __gpy_register_with_agent
        end
        return 0
    end

    if set -q __gpy_register_retry_pending
        set -e __gpy_register_retry_pending
        __gpy_register_with_agent
        return 0
    end

    if __gpy_supervisor_check; and not set -q __gpy_registered
        __gpy_register_with_agent
    end
    return 0
end

function __gpy_unregister_on_exit --on-event fish_exit
    __gpy_unregister_from_agent
    __gpy_untrack_shell_for_agent_recovery
end

function __gpy_workspace_on_pwd --on-variable PWD
    __gpy_sync_workspace
end
