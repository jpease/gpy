__gpy_supervisor_is_running() {
    local socket_path
    __gpy_ipc_endpoint_into socket_path

    [[ -S "$socket_path" ]] || return 1

    local ping_json='{"op":"ping"}'
    local response
    __gpy_send_json "$ping_json" "" response 2>/dev/null

    [[ -n "$response" && "$response" == *'"status":"ok"'* ]]
}

# Runs `gpy-agent start` unconditionally: it is idempotent and itself keeps a
# healthy agent or replaces one on another protocol or version (left running
# across an upgrade). Called once at shell init, and by the supervisor check
# when the agent is unreachable.
__gpy_supervisor_start() {
    # Not gated on GPY_AGENT_SUPERVISOR_ENABLED, which only controls restarts.
    __gpy_agent_enabled || return 0

    # Never waits: the first prompt renders from the cache or oneshot, and a
    # later prompt registers once the agent is up.
    if command -v gpy-agent &>/dev/null; then
        nohup gpy-agent start &>/dev/null &
        disown
        return 0
    fi

    return 1
}

# Per-prompt restart of a dead agent, rate limited and attempt-capped. The
# startup start (`__gpy_init`) counts as the first check, so the first probe
# runs one interval later.
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
    rate_limit=$((10#$rate_limit)) # zero-padded (08) is decimal, not octal

    # Builtin epoch on bash 4.2+: no fork for the checks the limiter rejects.
    local now
    __gpy_now_seconds now 2>/dev/null
    [[ "$now" =~ ^[0-9]+$ ]] || return

    if ((now - __gpy_supervisor_last_check_time < rate_limit)); then
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
    max_attempts=$((10#$max_attempts))
    if ((__gpy_supervisor_check_attempts >= max_attempts)); then
        return
    fi

    __gpy_supervisor_check_attempts=$((__gpy_supervisor_check_attempts + 1))
    __gpy_supervisor_start &>/dev/null &
    disown
}
