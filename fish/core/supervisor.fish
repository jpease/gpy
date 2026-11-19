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
# binary resolves; with no socket file that costs no fork. In an interactive
# shell the first prompt's attempt waits until that prompt is on screen
# (__gpy_defer_registration_past_paint). The workspace sync runs here too,
# once per prompt and only when $PWD changed.
function __gpy_supervisor_on_prompt --on-event fish_prompt --description 'Start the agent on the first prompt, then check it on later prompts'
    __gpy_agent_enabled; or return 0

    # A deferral still pending here never saw GPY's fish_prompt draw again
    # (another prompt function): this prompt registers as usual.
    set -e __gpy_register_deferred

    if not set -q __gpy_supervisor_initialized
        set -g __gpy_supervisor_initialized 1
        # The startup start counts as the first check.
        set -g __gpy_supervisor_last_check_time (__gpy_now_seconds)
        __gpy_start_agent_detached
        __gpy_defer_registration_past_paint
    else if not set -q __gpy_exec_declined; and __gpy_resolve_cli_binary >/dev/null
        # `gpy prompt` registers, syncs the workspace and supervises.
        return 0
    else
        __gpy_supervisor_check
    end

    # Asks nothing while the registration is deferred.
    if not set -q __gpy_registered
        __gpy_register_with_agent
    end
    __gpy_sync_workspace
    return 0
end

# Fish has no hook that runs once a prompt is drawn: the fish_prompt event
# and function both run before it. A readline function queued here
# (`commandline -f repaint`) runs only after the reader drew the prompt, and
# it calls fish_prompt again, which then registers
# (__gpy_register_after_paint). Until then __gpy_register_with_agent sends
# nothing, so neither this handler nor a segment holds the first paint with
# a round trip (and the agent's registration scan). A non-interactive Fish
# draws no prompt and registers at once.
function __gpy_defer_registration_past_paint --description 'Hold the first registration until the first prompt is drawn'
    status is-interactive; or return 1
    commandline -f repaint 2>/dev/null; or return 1
    set -g __gpy_register_deferred drawing
end

# fish_prompt calls this while a registration is deferred. Its first call
# draws the deferred prompt; the next one (the queued repaint) runs with
# that prompt on screen and registers.
function __gpy_register_after_paint --description 'Register once the first prompt is on screen'
    if test "$__gpy_register_deferred" = drawing
        set -g __gpy_register_deferred drawn
        return 0
    end
    set -e __gpy_register_deferred
    __gpy_register_with_agent >/dev/null 2>&1
    return 0
end

function __gpy_unregister_on_exit --on-event fish_exit
    # `gpy prompt` registers this shell, so __gpy_registered may be unset
    # while the agent lists it: always unregister (it untracks too).
    set -g __gpy_registered 1
    __gpy_unregister_from_agent
end
