# Caches only a confirmed protocol match, so health checks after the first pay
# for the ping alone; reset on every (re)start.
__gpy_protocol_version_checked=0

__gpy_supervisor_is_running() {
    local socket_path
    __gpy_ipc_endpoint_into socket_path

    [[ -S "$socket_path" ]] || return 1

    local ping_json='{"op":"ping"}'
    local response
    __gpy_send_json "$ping_json" "" response 2>/dev/null

    [[ -n "$response" && "$response" == *'"status":"ok"'* ]] || return 1

    # A responsive agent on the wrong protocol (left running across an
    # upgrade) counts as not running, so the caller restarts it.
    if [[ "$__gpy_protocol_version_checked" == "1" ]]; then
        return 0
    fi

    if __gpy_check_protocol_version; then
        __gpy_protocol_version_checked=1
        return 0
    fi

    return 1
}

__gpy_supervisor_start() {
    __gpy_protocol_version_checked=0

    # Not gated on GPY_AGENT_SUPERVISOR_ENABLED, which only controls restarts.
    __gpy_agent_enabled || return 0

    if __gpy_supervisor_is_running; then
        return 0
    fi

    # Never waits: the first prompt renders from the cache or oneshot, and a
    # later prompt registers once the agent is up.
    if command -v gpy-agent &>/dev/null; then
        nohup gpy-agent start &>/dev/null &
        disown
        return 0
    fi

    return 1
}

# Per-prompt restart of a dead agent, rate limited and attempt-capped.
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check_attempts=0

__gpy_supervisor_check() {
    __gpy_agent_enabled && __gpy_supervisor_enabled || return

    # Rate-limit before any IPC so a healthy agent costs no round-trips.
    # Read at call time so a config reload applies.
    # The user's non-negative integer (0 = every prompt), else the config
    # mirror, else 30.
    local rate_limit="${GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS-}"
    [[ "$rate_limit" =~ ^[0-9]+$ ]] || rate_limit="${__gpy_cfg_supervisor_check_interval-}"
    [[ "$rate_limit" =~ ^[0-9]+$ ]] || rate_limit=30

    # Builtin epoch on bash 4.2+: no fork for the checks the limiter rejects.
    local now
    __gpy_now_seconds now 2>/dev/null
    [[ "$now" =~ ^[0-9]+$ ]] || return

    if [[ $((now - __gpy_supervisor_last_check_time)) -lt $rate_limit ]]; then
        return
    fi

    __gpy_supervisor_last_check_time=$now

    if __gpy_supervisor_is_running; then
        __gpy_supervisor_check_attempts=0
        return
    fi

    # The user's non-negative integer, else the config mirror, else 5.
    local max_attempts="${GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS-}"
    [[ "$max_attempts" =~ ^[0-9]+$ ]] || max_attempts="${__gpy_cfg_supervisor_max_restarts-}"
    [[ "$max_attempts" =~ ^[0-9]+$ ]] || max_attempts=5
    if [[ $__gpy_supervisor_check_attempts -ge $max_attempts ]]; then
        return
    fi

    __gpy_supervisor_check_attempts=$((__gpy_supervisor_check_attempts + 1))
    # Reset here too: the backgrounded start runs in a subshell, so its own
    # reset never reaches this shell.
    __gpy_protocol_version_checked=0
    __gpy_supervisor_start &>/dev/null &
    disown
}
