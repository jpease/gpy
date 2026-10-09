# zsh/core/supervisor.zsh
# Agent lifecycle management
#
# Both functions are always defined. The enable flags come from the theme
# export (config.toml), which loads after this file, so they are read when
# each function acts, never at source time (#762). gpy.zsh makes the one
# startup __gpy_start_agent call after __gpy_load_theme, gated on
# GPY_AGENT_ENABLED alone: GPY_AGENT_SUPERVISOR_ENABLED=0 only turns off
# mid-session restarts (#842), and agent-free mode starts nothing (#841).

function __gpy_start_agent() {
    # Check if agent is already running. `gpy-agent status` exits 0 for any
    # responsive agent, including one speaking a mismatched wire protocol
    # (e.g. left running across an upgrade), so also check protocol version
    # directly and treat a mismatch as not-running -- otherwise this shell
    # would silently talk past a stale daemon indefinitely (#307).
    if gpy-agent status &>/dev/null && __gpy_check_protocol_version; then
        return 0
    fi

    # Start agent in background
    # We use &! to disown immediately in zsh
    gpy-agent start &>/dev/null &!

    # Wait briefly for startup. Bounded to roughly the same ~150ms IPC target
    # used elsewhere instead of 500ms: a present-but-unhealthy binary would
    # otherwise block every new shell for the entire window (#324). Success
    # still returns immediately.
    local max_wait=15  # 150ms total
    local i=0
    while [[ $i -lt $max_wait ]]; do
        if gpy-agent status &>/dev/null; then
            return 0
        fi
        sleep 0.01
        ((i++))
    done

    return 1
}

# Mid-session health check (#638), called from __gpy_precmd: Bash had
# __gpy_supervisor_check but Zsh's supervisor was startup-only, so an agent
# that died under an open Zsh shell was never restarted. Rate limited to one
# probe per [agent.supervisor] check_interval_seconds (default 30) and to
# max_restart_attempts restarts (default 5), read from the exported
# GPY_AGENT_SUPERVISOR_* values on every call (#762); the legacy
# GPY_SUPERVISOR_CHECK_* names stay as explicit overrides. The restart itself
# runs in the background so a prompt never waits on it.
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check_attempts=0
function __gpy_supervisor_check() {
    [[ "${GPY_AGENT_SUPERVISOR_ENABLED:-1}" == 1 && "${GPY_AGENT_ENABLED:-1}" == 1 ]] || return 0

    local rate_limit="${GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS:-${GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS:-30}}"
    [[ "$rate_limit" == <-> ]] || rate_limit=30
    local now=$EPOCHSECONDS
    (( now - __gpy_supervisor_last_check_time >= rate_limit )) || return 0
    __gpy_supervisor_last_check_time=$now

    # A live socket that answers status is healthy; reset the budget.
    if [[ -S "$(__gpy_ipc_endpoint)" ]] && gpy-agent status &>/dev/null; then
        __gpy_supervisor_check_attempts=0
        return 0
    fi

    local max_attempts="${GPY_SUPERVISOR_CHECK_MAX_ATTEMPTS:-${GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS:-5}}"
    [[ "$max_attempts" == <-> ]] || max_attempts=5
    (( __gpy_supervisor_check_attempts < max_attempts )) || return 0
    (( __gpy_supervisor_check_attempts++ ))
    __gpy_start_agent &>/dev/null &!
}
