# SPDX-License-Identifier: GPL-3.0-or-later

source (path dirname (status filename))/flags.fish

function __gpy_config_root
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        echo "$XDG_CONFIG_HOME/fish"
    else
        echo "$HOME/.config/fish"
    end
end

# PATH first, then ~/.local/bin; never cached (the lookup is about 50 us).
function __gpy_resolve_agent_binary --description 'Resolve the gpy-agent binary path'
    set -l command_path (command -s gpy-agent)
    if test -n "$command_path"
        echo "$command_path"
        return 0
    end

    if test -x "$HOME/.local/bin/gpy-agent"
        echo "$HOME/.local/bin/gpy-agent"
        return 0
    end

    return 1
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

# Epoch time in ms (second resolution); reuses $__gpy_prompt_now (set once per render) to avoid forking date.
function __gpy_get_time_ms
    if set -q __gpy_prompt_now; and string match -qr '^[0-9]+$' -- "$__gpy_prompt_now"
        math "$__gpy_prompt_now * 1000"
    else
        math (date +%s) \* 1000
    end
end

function __has_binary --argument-names bin
    set -l cache_var __gpy_has_"$bin"_bin
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
# Rust counterpart: `paths::cache_root_for`.
function __gpy_theme_export_cache_path --description 'Path to the theme export cache file'
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy/theme-export.fish"
    else
        set -l home (__gpy_home)
        test -n "$home"; and echo "$home/.cache/gpy/theme-export.fish"
    end
end

function __gpy_log_error --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: $message" >&2
    end
end

function __gpy_log_warn --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: warning: $message" >&2
    end
end

function __gpy_log_debug --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: debug: $message" >&2
    end
end

function __gpy_log_info --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: info: $message" >&2
    end
end

# Pure policy helpers: no fork, clock, filesystem, or globals (unit-tested).

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

# Epoch seconds without a fork: mtime of / plus the seconds since it.
function __gpy_now_seconds
    set -l at (path mtime / 2>/dev/null)
    set -l age (path mtime --relative / 2>/dev/null)
    if string match -qr '\A[0-9]+\z' -- "$at"; and string match -qr '\A-?[0-9]+\z' -- "$age"
        math "$at + $age"
        return 0
    end
    date +%s 2>/dev/null
end

function __gpy_agent_available
    if not __gpy_agent_enabled
        return 1
    end

    if not __gpy_resolve_agent_binary >/dev/null
        return 1
    end

    # Check output, not exit code: an empty reply still exits 0.
    set -l ping_result (__gpy_request ping 2>/dev/null)
    test -n "$ping_result"
end
