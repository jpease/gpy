# SPDX-License-Identifier: GPL-3.0-or-later

function __gpy_config_root
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        echo "$XDG_CONFIG_HOME/fish"
    else
        echo "$HOME/.config/fish"
    end
end

function __gpy_resolve_agent_binary --description 'Resolve the gpy-agent binary path'
    if set -q GPY_AGENT_BINARY_PATH; and test -x "$GPY_AGENT_BINARY_PATH"
        echo "$GPY_AGENT_BINARY_PATH"
        return 0
    end

    if command -q gpy-agent
        set -l command_path (command -s gpy-agent)
        if test -n "$command_path"
            set -g GPY_AGENT_BINARY_PATH "$command_path"
            echo "$command_path"
            return 0
        end
    end

    if test -x "$HOME/.local/bin/gpy-agent"
        set -g GPY_AGENT_BINARY_PATH "$HOME/.local/bin/gpy-agent"
        echo "$HOME/.local/bin/gpy-agent"
        return 0
    end

    return 1
end

# Mirrors `config::schema::get_config_paths`; relative GPY_CONFIG_PATH joins the physical cwd.
function __gpy_user_config_candidates
    set -l candidates
    if set -q GPY_CONFIG_PATH; and test -n "$GPY_CONFIG_PATH"
        if string match -q '/*' -- "$GPY_CONFIG_PATH"
            set candidates $GPY_CONFIG_PATH
        else
            set candidates (string trim -r -c / -- (pwd -P))/$GPY_CONFIG_PATH
        end
    end
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"; and string match -q '/*' -- "$XDG_CONFIG_HOME"
        set -a candidates "$XDG_CONFIG_HOME/gpy/config.toml"
    end
    set -l home (__gpy_home)
    if test -n "$home"
        set -a candidates "$home/.config/gpy/config.toml"
    end
    printf '%s\n' $candidates
end

function __gpy_locate_config_path
    set -l candidates (__gpy_user_config_candidates)
    for path in $candidates
        if test -e $path
            echo $path
            return
        end
    end
    if test (count $candidates) -gt 0
        echo $candidates[1]
    end
end

function __gpy_file_mtime --argument-names file
    # `path mtime` is a builtin: no fork.
    path mtime "$file" 2>/dev/null
end

# Fork-free byte-exact read; `read -z` keeps trailing newlines.
function __gpy_read_file --argument-names file
    read -zl content <"$file"; or return 1
    printf '%s' $content
end

# Milliseconds, reusing the per-render $__gpy_prompt_now to avoid forking `date`.
# Reuses $__gpy_prompt_now (captured once per render in fish_prompt) to avoid
function __gpy_get_time_ms
    if set -q __gpy_prompt_now; and string match -qr '^[0-9]+$' -- "$__gpy_prompt_now"
        math "$__gpy_prompt_now * 1000"
    else
        math (date +%s) \* 1000
    end
end

function __has_binary --argument-names bin
    set -l cache_var __has_"$bin"_bin
    if not set -q $cache_var
        if command -q $bin
            set -g $cache_var 1
        else
            set -g $cache_var 0
        end
    end
    test $$cache_var -eq 1
end

# The agent writes this before ringing the reload doorbell.
function __gpy_theme_export_cache_path --description 'Path to the theme export cache file'
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy/theme-export.fish"
    else
        set -l home (__gpy_home)
        test -n "$home"; and echo "$home/.cache/gpy/theme-export.fish"
    end
end

function __gpy_log_error --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: $message" >&2
    end
end

function __gpy_log_warn --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: warning: $message" >&2
    end
end

function __gpy_log_debug --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: debug: $message" >&2
    end
end

function __gpy_log_info --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: info: $message" >&2
    end
end

# Pure policy helpers: no fork, clock, filesystem, or globals (unit-tested).

# Prints deny iff both are integers and backoff_until > now; otherwise allow.
function __gpy_breaker_gate --argument-names backoff_until now --description 'Pure circuit-breaker gate: allow|deny'
    set -q backoff_until[1]; or set backoff_until ""
    set -q now[1]; or set now ""
    if string match -qr '^[0-9]+$' -- "$backoff_until"
        and string match -qr '^[0-9]+$' -- "$now"
        and test "$backoff_until" -gt "$now"
        echo deny
        return 0
    end
    echo allow
end

# Prints "<new_count> <new_until>"; opening the breaker resets the count. Unknown now counts as 0.
function __gpy_breaker_record_failure --argument-names failure_count now threshold backoff_secs --description 'Pure circuit-breaker failure accounting: prints "<new_count> <new_until>"'
    set -q failure_count[1]; or set failure_count ""
    if not string match -qr '^[0-9]+$' -- "$failure_count"
        set failure_count 0
    end
    set -l new_count (math "$failure_count + 1")
    if test "$new_count" -ge "$threshold"
        set -l base 0
        string match -qr '^[0-9]+$' -- "$now"; and set base "$now"
        printf '%s %s\n' 0 (math "$base + $backoff_secs")
    else
        printf '%s %s\n' "$new_count" 0
    end
end

# stale iff all operands are integers and now - mtime >= ttl.
function __gpy_cache_freshness --argument-names now mtime ttl --description 'Pure cache freshness check: fresh|stale'
    if string match -qr '^[0-9]+$' -- "$now"
        and string match -qr '^[0-9]+$' -- "$mtime"
        and string match -qr '^[0-9]+$' -- "$ttl"
        and test (math "$now - $mtime") -ge "$ttl"
        echo stale
        return 0
    end
    echo fresh
end

# Exit 0 iff interval_ms elapsed; empty last_ms counts as 0.
function __gpy_throttle_elapsed --argument-names last_ms now_ms interval_ms --description 'Pure throttle check: exit 0 iff interval_ms has elapsed since last_ms'
    set -q last_ms[1]; or set last_ms ""
    test -z "$last_ms"; and set last_ms 0
    test (math "$now_ms - $last_ms") -ge "$interval_ms"
end

# Status codes from __gpy_read_instant_cache: bit 2 = stale, bit 4 = variant fallback.
function __gpy_cache_status_stale --argument-names code --description 'Pure predicate: does this cache-read status code indicate staleness?'
    test "$code" = 2 -o "$code" = 6
end

function __gpy_cache_status_variant --argument-names code --description 'Pure predicate: does this cache-read status code indicate a .none variant-fallback read?'
    test "$code" = 4 -o "$code" = 6
end

set -g __gpy_agent_failure_count 0
set -g __gpy_agent_backoff_until 0

# Ping with a circuit breaker; forks `date` only while the breaker may be open.
function __gpy_agent_available
    if test "$GPY_AGENT_ENABLED" = 0
        return 1
    end

    if not __gpy_resolve_agent_binary >/dev/null
        return 1
    end

    # Circuit breaker: skip date fork entirely when backoff is not active (common case)
    if test "$__gpy_agent_backoff_until" -gt 0
        # Fresh timestamp: __gpy_prompt_now is stale in idle shells and would keep backoff active.
        set -l _now_check (date +%s 2>/dev/null)
        if test (__gpy_breaker_gate "$__gpy_agent_backoff_until" "$_now_check") = deny
            return 1
        end
        set -g __gpy_agent_backoff_until 0
    end

    # Check output, not exit code: an empty reply still exits 0.
    set -l ping_result (__gpy_request ping 2>/dev/null)
    if test -n "$ping_result"
        set -g __gpy_agent_failure_count 0
        return 0
    end

    # Fork `date` only when this failure would open the breaker.
    set -l _now_backoff ""
    if test (math "$__gpy_agent_failure_count + 1") -ge "$GPY_CIRCUIT_BREAKER_THRESHOLD"
        set _now_backoff (date +%s 2>/dev/null)
    end
    set -l _breaker_result (__gpy_breaker_record_failure "$__gpy_agent_failure_count" "$_now_backoff" "$GPY_CIRCUIT_BREAKER_THRESHOLD" "$GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS")
    set -l _breaker_fields (string split ' ' -- $_breaker_result)
    set -g __gpy_agent_failure_count $_breaker_fields[1]
    set -g __gpy_agent_backoff_until $_breaker_fields[2]

    if test "$__gpy_agent_backoff_until" -gt 0
        __gpy_log_debug ipc "Circuit breaker opened: backing off for $GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS seconds"
    end

    return 1
end
