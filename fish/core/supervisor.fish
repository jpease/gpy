# SPDX-License-Identifier: GPL-3.0-or-later

# Live supervisor, not merely a live (possibly recycled) PID.
function __gpy_is_supervisor_pid --argument-names pid
    string match -qr '^\d+$' -- "$pid"; or return 1
    ps -o command= -p $pid 2>/dev/null | string match -q '*__gpy_agent_supervisor_loop*'
end

function __gpy_agent_supervisor_start --description 'Start agent supervisor for automatic restart'
    if not __gpy_supervisor_enabled; or not __gpy_agent_enabled
        return 0
    end

    if not __gpy_resolve_agent_binary >/dev/null
        __gpy_log_warn ipc "Agent binary not found in PATH; skipping supervisor"
        return 1
    end

    set -l runtime_root (__gpy_runtime_root)
    mkdir -p "$runtime_root"
    set -l supervisor_pidfile "$runtime_root/supervisor.pid"

    # Never remove the file here: only the child recovers it, under the claim.
    if __gpy_is_supervisor_pid (cat "$supervisor_pidfile" 2>/dev/null)
        return 0
    end

    set -l config_root (__gpy_config_root)
    # fds are redirected at spawn: a redirect-only `exec` in fish does not
    # reopen the shell's fds. The child claims the pidfile with no-clobber `>?`
    # (O_EXCL); an empty read means the winner hasn't written its PID yet.
    set -l supervisor_script '
        cd /
        set -gx GPY_SUPERVISOR_CHILD 1
        set -l pidfile $argv[1]
        set -l cfgroot $argv[2]
        source "$cfgroot/gpy/core/constants.fish"; or exit 1
        source "$cfgroot/gpy/core/util.fish"; or exit 1
        source "$cfgroot/gpy/core/ipc.fish"; or exit 1
        source "$cfgroot/gpy/core/supervisor.fish"; or exit 1
        if not echo $fish_pid >?"$pidfile"
            set -l owner (cat "$pidfile" 2>/dev/null)
            if test -z "$owner"
                sleep 0.2
                set owner (cat "$pidfile" 2>/dev/null)
            end
            __gpy_is_supervisor_pid $owner; and exit 0
            rm -f "$pidfile"
            echo $fish_pid >?"$pidfile"; or exit 0
        end
        __gpy_agent_supervisor_loop "$pidfile"
        test "$(cat "$pidfile" 2>/dev/null)" = "$fish_pid"; and rm -f "$pidfile"
    '

    fish -c "$supervisor_script" -- "$supervisor_pidfile" "$config_root" </dev/null >/dev/null 2>&1 &
    set -l spawned_pid $last_pid
    disown $spawned_pid 2>/dev/null

    if test -n "$spawned_pid"
        __gpy_log_debug ipc "Agent supervisor started with PID $spawned_pid"
        return 0
    end

    __gpy_log_error ipc "Failed to start agent supervisor loop"
    return 1
end

# The detached supervisor sees config changes only via the export cache;
# read just the enable flags and its own timing.
function __gpy_supervisor_refresh_settings --description 'Re-read the supervisor settings from the theme-export cache'
    set -l cache_path (__gpy_theme_export_cache_path)
    test -n "$cache_path" -a -r "$cache_path"; or return 0
    set -l pairs (string match -rg '^set -gx (GPY_AGENT_ENABLED|GPY_AGENT_SUPERVISOR_ENABLED|GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS|GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS) "(\d+)"$' <"$cache_path")
    while set -q pairs[2]
        set -gx $pairs[1] $pairs[2]
        set -e pairs[1..2]
    end
end

# Exits once $argv[1] (the claimed pidfile) no longer names this process.
function __gpy_agent_supervisor_loop --description 'Background loop for agent supervision'
    set -l pidfile $argv[1]

    set -l restart_count 0
    set -l last_restart_time 0
    # A stall shorter than one check interval clears by itself, so an agent
    # that is connected but silent is restarted only after this many checks in
    # a row; an unreachable one is restarted at once.
    set -l unresponsive_limit 2
    set -l unresponsive_checks 0

    while true
        # Re-read each pass so a config reload applies within one interval:
        # disabling stops the loop, new timing governs the next sleep.
        __gpy_supervisor_refresh_settings
        if not __gpy_supervisor_enabled; or not __gpy_agent_enabled
            break
        end
        # Re-validate: an empty value would make sleep busy-loop.
        set -l check_interval (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)
        set -l max_restart_attempts (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)

        if test -n "$pidfile"; and test "$(cat "$pidfile" 2>/dev/null)" != "$fish_pid"
            break
        end

        if not __gpy_resolve_agent_binary >/dev/null
            sleep $check_interval; or break
            continue
        end

        __gpy_agent_is_healthy
        set -l health $status
        if test $health -eq 2
            set unresponsive_checks (math $unresponsive_checks + 1)
        else
            set unresponsive_checks 0
        end

        if test $health -eq 0
            set restart_count 0
        else if test $health -eq 1; or test $unresponsive_checks -ge $unresponsive_limit
            set -l current_time (date +%s)

            set -l rate_limit (__gpy_uint_or_default "$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS" 60)

            if test (math "$current_time - $last_restart_time") -gt $rate_limit
                if test $restart_count -lt $max_restart_attempts
                    __gpy_log_warn ipc "Agent not responding, attempting restart ($restart_count/$max_restart_attempts)"
                    set unresponsive_checks 0
                    if __gpy_agent_restart
                        __gpy_log_info ipc "Agent restarted successfully"
                        set restart_count 0
                        set last_restart_time $current_time
                    else
                        __gpy_log_error ipc "Agent restart failed"
                        set restart_count (math "$restart_count + 1")
                        set last_restart_time $current_time
                    end
                else
                    set -l long_backoff (__gpy_uint_or_default "$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS" 300)
                    __gpy_log_error ipc "Agent restart limit exceeded, backing off"
                    sleep $long_backoff; or break
                    set restart_count 0
                    continue
                end
            end
        end

        sleep $check_interval; or break
    end
end

# 0: healthy. 1: unreachable (no socket, nothing listening). 2: connected but
# no valid reply, which a briefly stalled agent also produces. Nothing waits on
# this background check, so it allows more than the prompt's reply budget.
function __gpy_agent_is_healthy --description 'Check the agent: 0 healthy, 1 unreachable, 2 unresponsive'
    set -l prompt_budget (__gpy_uint_or_default "$GPY_IPC_TIMEOUT_MS" 150)
    set -l response (__gpy_ipc_send '{"op":"ping"}' (math "max($prompt_budget, 1000)"))
    test $status -eq 1; and return 1

    if string match -q '*"status":"ok"*' -- "$response"
        return 0
    end
    return 2
end

# Socket-holder PIDs via lsof or fuser; returns 1 when neither exists, which the
# caller must not read as a stale socket.
function __gpy_socket_owner_pids --argument-names socket_path
    if command -q lsof
        lsof -t -- "$socket_path" 2>/dev/null
        return 0
    else if command -q fuser
        # psmisc prints PIDs on stdout (the path goes to stderr); BSD fuser
        # prints "path: pid ..." there, so keep only pure-digit words.
        string match -r '^[0-9]+$' -- (string split ' ' -- (string trim -- (fuser -- "$socket_path" 2>/dev/null)))
        return 0
    end
    return 1
end

function __gpy_agent_restart --description 'Restart the agent process'
    set -l socket_path (__gpy_ipc_endpoint)
    set -l agent_binary (__gpy_resolve_agent_binary)

    # Only with a live socket: on cold start the CLI prints a notice to stdout.
    if test -n "$agent_binary"; and test -S "$socket_path"
        "$agent_binary" stop >/dev/null 2>&1
        sleep 0.5
    end

    # Without lsof/fuser, leave the socket: deleting a live agent's socket
    # would orphan it, and `gpy-agent start` reclaims stale ones.
    if test -S "$socket_path"
        set -l socket_pid (__gpy_socket_owner_pids "$socket_path")
        if test $status -ne 0
            __gpy_log_warn ipc "neither lsof nor fuser found; cannot stop a lingering agent holding $socket_path (install lsof)"
        else
            if test -n "$socket_pid"
                kill -TERM $socket_pid 2>/dev/null
                sleep 0.3
                if kill -0 $socket_pid 2>/dev/null
                    kill -KILL $socket_pid 2>/dev/null
                    sleep 0.1
                end
            end
            rm -f "$socket_path" 2>/dev/null
        end
    end

    if test -n "$agent_binary"
        "$agent_binary" start >/dev/null 2>&1

        if __gpy_wait_for_socket "$socket_path"
            return 0
        end
    end

    return 1
end

# Defer supervisor start to the first prompt to avoid blocking source.
if not set -q GPY_SUPERVISOR_CHILD; and begin
        __gpy_agent_enabled; or __gpy_supervisor_enabled
    end
    function __gpy_start_supervisor_on_prompt --on-event fish_prompt
        if __gpy_agent_enabled
            # Only the restart loop depends on the supervisor flag.
            if not set -q __gpy_supervisor_initialized
                set -g __gpy_supervisor_initialized 1
                not __gpy_supervisor_enabled; or __gpy_agent_supervisor_start
            end

            # An agent speaking another protocol must be treated as not running.
            if not set -q __gpy_protocol_checked
                set -g __gpy_protocol_checked 1
                if __gpy_agent_available; and not __gpy_check_protocol_version
                    __gpy_log_warn ipc "Protocol version mismatch detected; restarting agent"
                    __gpy_agent_restart
                end
            end

            if not __gpy_agent_available
                set -l max_attempts (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_MAX_ATTEMPTS" 3)

                set -l attempts 0
                if set -q __gpy_autostart_attempts
                    set attempts $__gpy_autostart_attempts
                end

                if test $attempts -ge $max_attempts
                    # Crash loop: leave retries to the supervisor.
                    __gpy_log_error ipc "Agent autostart limit exceeded ($max_attempts attempts); disabling per-prompt autostart for this shell"
                    functions -e __gpy_start_supervisor_on_prompt
                    return
                end

                set -l rate_limit (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS" 10)

                set -l last_attempt_time 0
                if set -q __gpy_autostart_last_attempt_time
                    set last_attempt_time $__gpy_autostart_last_attempt_time
                end

                set -l current_time (date +%s)
                if test (math "$current_time - $last_attempt_time") -ge $rate_limit
                    set -g __gpy_autostart_last_attempt_time $current_time
                    set -g __gpy_autostart_attempts (math "$attempts + 1")

                    set -l agent_binary (__gpy_resolve_agent_binary)
                    if test -n "$agent_binary"
                        "$agent_binary" start >/dev/null 2>&1 &
                        disown $last_pid 2>/dev/null
                        sleep (math "$GPY_AGENT_START_DELAY_MS / 1000")
                    end
                end
            end

            if __gpy_register_with_agent
                functions -e __gpy_start_supervisor_on_prompt
                set -e __gpy_autostart_attempts
                set -e __gpy_autostart_last_attempt_time
            end
        else
            functions -e __gpy_start_supervisor_on_prompt
        end
    end

    function __gpy_unregister_on_exit --on-event fish_exit
        __gpy_unregister_from_agent
        __gpy_untrack_shell_for_agent_recovery
    end

    function __gpy_workspace_on_pwd --on-variable PWD
        __gpy_sync_workspace
    end
end
