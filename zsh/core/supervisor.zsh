# zsh/core/supervisor.zsh
# Enable flags come from the theme export, which loads after this file, so
# they are read at call time, never at source time.

function __gpy_start_agent() {
    # `status` succeeds for an agent on a mismatched protocol too (e.g. left
    # running across an upgrade); treat that as not running.
    if gpy-agent status &>/dev/null && __gpy_check_protocol_version; then
        return 0
    fi

    # &! disowns immediately. Never waits: the first prompt renders from the
    # cache or oneshot, and the post-render retry registers once the agent is up.
    gpy-agent start &>/dev/null &!
    return 0
}

# Mid-session health check from precmd, rate limited by check interval and
# capped at max restart attempts.
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check_attempts=0
function __gpy_supervisor_check() {
    __gpy_agent_enabled && __gpy_supervisor_enabled || return 0

    local rate_limit="${GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS:-30}"
    [[ "$rate_limit" == <-> ]] || rate_limit=30
    local now=$EPOCHSECONDS
    (( now - __gpy_supervisor_last_check_time >= rate_limit )) || return 0
    __gpy_supervisor_last_check_time=$now

    if [[ -S "$(__gpy_ipc_endpoint)" ]] && gpy-agent status &>/dev/null; then
        __gpy_supervisor_check_attempts=0
        return 0
    fi

    local max_attempts="${GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS:-5}"
    [[ "$max_attempts" == <-> ]] || max_attempts=5
    (( __gpy_supervisor_check_attempts < max_attempts )) || return 0
    (( __gpy_supervisor_check_attempts++ ))
    __gpy_start_agent &>/dev/null &!
}
