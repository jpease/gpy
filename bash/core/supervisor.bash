# bash/core/supervisor.bash
# Agent supervisor - auto-start and health check

# Caches a successful #307 protocol-version check for the current agent
# instance so repeat health checks only pay for the ping, not a second
# `{"op":"status"}` round-trip (#340). A confirmed MISMATCH is never cached
# (see __gpy_supervisor_is_running below) and __gpy_supervisor_start resets
# this to 0 so a freshly (re)started agent is always re-validated.
__gpy_protocol_version_checked=0

# Check if agent is running
__gpy_supervisor_is_running() {
    local socket_path
    socket_path=$(__gpy_ipc_endpoint)

    # Check if socket exists
    [[ -S "$socket_path" ]] || return 1

    # Try to ping agent
    local ping_json='{"op":"ping"}'
    local response
    response=$(__gpy_send_json "$ping_json" 2>/dev/null)

    # Check for valid pong response
    # The daemon answers a ping with its JSON acknowledgement,
    # {"status":"ok"} (the shape every shell's registration check matches).
    # This used to look for an invented `"op":"pong"` reply the daemon has
    # never sent, so a live agent was never recognised: every new shell
    # respawned `gpy-agent start` and the mid-session check burned its
    # attempts on a healthy daemon (#638).
    [[ -n "$response" && "$response" == *'"status":"ok"'* ]] || return 1

    # A responsive agent speaking the wrong wire protocol (e.g. an old
    # daemon left running across an upgrade) must be treated as not
    # running so the caller restarts it onto a matching version (#307).
    # The protocol check is a second IPC round-trip; once it has succeeded
    # for this agent instance, skip it on subsequent health checks so a
    # healthy agent only pays the ping cost (#340). Never cache a mismatch --
    # only a confirmed match -- so a mismatched-but-responsive agent is never
    # sticky-cached as healthy.
    if [[ "$__gpy_protocol_version_checked" == "1" ]]; then
        return 0
    fi

    if __gpy_check_protocol_version; then
        __gpy_protocol_version_checked=1
        return 0
    fi

    return 1
}

# Start the agent
__gpy_supervisor_start() {
    # Invalidate the protocol-version cache so a newly (re)started agent is
    # always re-validated instead of inheriting a prior instance's cached
    # success -- preserves the #307 guarantee under the #340 caching change.
    # This function runs both at fresh shell startup (init.bash) and on the
    # down-path restart (__gpy_supervisor_check below), so resetting here
    # covers both callers.
    __gpy_protocol_version_checked=0

    # Check if supervisor is disabled
    if [[ -n "${GPY_AGENT_SUPERVISOR_ENABLED:-}" && "${GPY_AGENT_SUPERVISOR_ENABLED:-}" != "1" ]]; then
        return
    fi

    # Check if agent is already running
    if __gpy_supervisor_is_running; then
        return 0
    fi

    # Try to start agent
    if command -v gpy-agent &>/dev/null; then
        # Start in background, detached from shell
        nohup gpy-agent start &>/dev/null &
        disown

        # Wait briefly for agent to start. Bounded to roughly the same ~150ms
        # IPC target used elsewhere instead of a full second: a present-but-
        # unhealthy binary would otherwise block every new shell for the
        # entire window (#324). Success still returns immediately.
        local max_attempts=3
        local attempt=0

        while [[ $attempt -lt $max_attempts ]]; do
            sleep 0.05

            if __gpy_supervisor_is_running; then
                return 0
            fi

            attempt=$((attempt + 1))
        done

        # Agent failed to start within timeout
        return 1
    fi

    # gpy-agent not found
    return 1
}

# Supervisor health check, wired into the per-prompt precmd hook (#324) so a
# mid-session-dead agent gets restarted instead of staying dead for the rest
# of the shell session. Rate limited and attempt-capped so a persistently
# unhealthy agent doesn't fork a restart attempt on every single prompt.
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check_attempts=0

__gpy_supervisor_check() {
    # Only check if supervisor is enabled
    if [[ -z "${GPY_AGENT_SUPERVISOR_ENABLED:-}" || "${GPY_AGENT_SUPERVISOR_ENABLED:-}" != "1" ]]; then
        return
    fi

    # Rate-limit gate FIRST, before any IPC round-trip (#340). The healthy
    # path used to pay for a ping (and a second protocol-version round-trip)
    # on every single prompt because __gpy_supervisor_is_running was called
    # unconditionally as the first action here; only the restart itself was
    # rate-limited. Checking the window before touching the socket means a
    # healthy agent costs zero round-trips within the window, and the
    # down/restart path below is reached -- and thus pings -- at most once
    # per window, same cadence as before. The interval and attempt cap come
    # from the theme export ([agent.supervisor] in config.toml) and are read
    # here, not at source time, so a reload applies (#762). The legacy
    # GPY_SUPERVISOR_CHECK_* names stay as explicit overrides.
    local rate_limit="${GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS:-${GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS:-30}}"
    [[ "$rate_limit" =~ ^[0-9]+$ ]] || rate_limit=30

    local now
    now=$(date +%s 2>/dev/null)
    [[ "$now" =~ ^[0-9]+$ ]] || return

    if [[ $((now - __gpy_supervisor_last_check_time)) -lt $rate_limit ]]; then
        return
    fi

    __gpy_supervisor_last_check_time=$now

    if __gpy_supervisor_is_running; then
        __gpy_supervisor_check_attempts=0
        return
    fi

    # Note the accepted behavior change (#340): a mid-session-dead agent is
    # now noticed up to $rate_limit seconds later instead of on the very next
    # prompt, since the ping above only runs after the window has elapsed.
    local max_attempts="${GPY_SUPERVISOR_CHECK_MAX_ATTEMPTS:-${GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS:-5}}"
    [[ "$max_attempts" =~ ^[0-9]+$ ]] || max_attempts=5
    if [[ $__gpy_supervisor_check_attempts -ge $max_attempts ]]; then
        return
    fi

    __gpy_supervisor_check_attempts=$((__gpy_supervisor_check_attempts + 1))
    # Invalidate the protocol cache in THIS (parent) shell before the restart.
    # __gpy_supervisor_start runs backgrounded (`&`), i.e. in a subshell, so
    # its own top-of-function `__gpy_protocol_version_checked=0` reset never
    # propagates back here. Without this parent-side reset the parent would
    # keep a stale cached "1" and skip the #307 protocol check on whatever
    # agent takes over the socket after the restart -- defeating the
    # invalidation the restart is supposed to guarantee.
    __gpy_protocol_version_checked=0
    __gpy_supervisor_start &>/dev/null &
    disown
}
