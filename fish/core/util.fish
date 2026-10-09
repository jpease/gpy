# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# UTILITY FUNCTIONS
# ============================================================================

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

    set -l legacy_path (__gpy_config_root)/gpy/bin/gpy-agent
    if test -x "$legacy_path"
        set -g GPY_AGENT_BINARY_PATH "$legacy_path"
        echo "$legacy_path"
        return 0
    end

    return 1
end

# Mirrors `config::schema::get_config_paths`: no project-local `.gpy.toml`
# (#733), and a relative GPY_CONFIG_PATH is joined onto the physical cwd the
# way the agent joins it onto `std::env::current_dir()`.
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
    # `path mtime` is a Fish built-in (no subprocess): returns the file's
    # modification time as a unix timestamp and exits non-zero for missing files.
    path mtime "$file" 2>/dev/null
end

# Emit a file's contents to stdout fork-free (no `cat`), preserving bytes exactly.
# Uses NUL-delimited `read -z` so trailing newlines are kept and empty files emit
# nothing. Single source of truth for the per-prompt instant-cache read path.
function __gpy_read_file --argument-names file
    read -zl content <"$file"; or return 1
    printf '%s' $content
end

# Get current time in milliseconds.
# Reuses $__gpy_prompt_now (captured once per render in fish_prompt) to avoid
# forking `date` on the warm prompt path. Falls back to `date +%s` when called
# outside a prompt render (e.g., init hooks). Callers that need sub-second
# precision for non-throttling purposes should use `date` directly.
function __gpy_get_time_ms
    if set -q __gpy_prompt_now; and string match -qr '^[0-9]+$' -- "$__gpy_prompt_now"
        math "$__gpy_prompt_now * 1000"
    else
        math (date +%s) \* 1000
    end
end

# Lazy binary availability cache
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

# ============================================================================
# CACHE PATHS
# ============================================================================
#
# `gpy_config_runtime_dir` and `gpy_config_path` used to live here: a THIRD
# runtime-root implementation (its own `/tmp/gpy-$USER` fallback, disagreeing
# with both `__gpy_runtime_root` and the agent) pointing at a `config.json`
# snapshot nothing in the agent has ever written. Their only readers tested
# for a file that could not exist and created a directory nothing used, so
# #626 deleted all of it rather than teaching a fourth resolver the shared
# precedence. `__gpy_runtime_root` (fish/core/ipc.fish) is the runtime root;
# `__gpy_locate_config_path` is the config file.

# Return the path to the cached Fish-format theme export.
# The agent writes this file before ringing the reload doorbell, so shells can source it
# without spawning the binary on the startup/reload hot path.
function __gpy_theme_export_cache_path --description 'Path to the theme export cache file'
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy/theme-export.fish"
    else
        set -l home (__gpy_home)
        test -n "$home"; and echo "$home/.cache/gpy/theme-export.fish"
    end
end

# ============================================================================
# ERROR HANDLING
# ============================================================================

# Consistent error reporting function
function __gpy_log_error --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: $message" >&2
    end
end

# Log warning messages
function __gpy_log_warn --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: warning: $message" >&2
    end
end

# Log debug messages
function __gpy_log_debug --argument-names context message
    if test "$GPY_DEBUG" = 1
        echo "gpy[$context]: debug: $message" >&2
    end
end

# Log info messages
function __gpy_log_info --argument-names context message
    if test "$GPY_DEBUG" = 1; or test "$GPY_VERBOSE" = 1
        echo "gpy[$context]: info: $message" >&2
    end
end

# ============================================================================
# PURE POLICY HELPERS (#612)
# ============================================================================
# No fork, no clock read, no filesystem access, no globals. Each takes every
# input it needs as an argument and prints/exits a pure function of those
# arguments, so each has a Fish unit test with no socket, clock, or
# filesystem (tests/fish/policy_helpers.test.fish). Callers own the actual
# fork/IPC/filesystem work and just plug in the results.

# Circuit-breaker gate: given the currently-recorded backoff deadline and a
# timestamp, decide whether to allow a real ping ("allow") or short-circuit
# it ("deny"). Deny iff BOTH are non-negative integers and backoff_until is
# strictly after now. A non-numeric/empty `now` (e.g. `date` failed) or a
# non-numeric/empty `backoff_until` falls through to "allow" -- matching
# today's "fall through to a real ping" behavior when the timestamp fork
# fails.
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

# Circuit-breaker failure accounting: given the failure count BEFORE this
# failure, the current timestamp, the threshold, and the backoff duration,
# print "<new_count> <new_until>". Increments the count; once the
# incremented count reaches `threshold` the breaker opens (prints
# "0 <now+backoff_secs>", resetting the count -- today's behavior), otherwise
# prints "<new_count> 0". A non-numeric/empty `now` when opening is treated
# as 0 (so `new_until` becomes just `backoff_secs`) rather than failing --
# still opens the breaker, just without crediting elapsed time it has no way
# to measure.
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

# Cache-freshness check: "fresh" or "stale", given a read timestamp, the
# cache entry's mtime, and its TTL (all seconds). "stale" iff all three are
# known integers and `now - mtime >= ttl`; any empty/non-numeric operand is
# "fresh" -- matching today's "only flag stale when both timestamps are
# known".
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

# Throttle check: exit 0 iff at least `interval_ms` have elapsed between
# `last_ms` and `now_ms`. An empty/unset `last_ms` (no previous stamp
# recorded yet) counts as 0, so the very first call always elapses.
function __gpy_throttle_elapsed --argument-names last_ms now_ms interval_ms --description 'Pure throttle check: exit 0 iff interval_ms has elapsed since last_ms'
    set -q last_ms[1]; or set last_ms ""
    test -z "$last_ms"; and set last_ms 0
    test (math "$now_ms - $last_ms") -ge "$interval_ms"
end

# Predicates over __gpy_read_instant_cache's exit-code status contract
# (0 fresh hit, 1 miss, 2 stale hit, 4 fresh variant-fallback hit, 6 stale
# variant-fallback hit -- bit 2 = stale, bit 4 = variant fallback). Callers
# use these instead of doing arithmetic on the raw code themselves.
function __gpy_cache_status_stale --argument-names code --description 'Pure predicate: does this cache-read status code indicate staleness?'
    test "$code" = 2 -o "$code" = 6
end

function __gpy_cache_status_variant --argument-names code --description 'Pure predicate: does this cache-read status code indicate a .none variant-fallback read?'
    test "$code" = 4 -o "$code" = 6
end

# ============================================================================
# AGENT COMMUNICATION
# ============================================================================

# Circuit breaker state for agent availability checks
set -g __gpy_agent_failure_count 0
set -g __gpy_agent_backoff_until 0

# Check if GPY agent is available and responding
# Implements simple circuit breaker to avoid repeated timeouts. The gate and
# failure-accounting decisions are pure helpers above (tested in
# tests/fish/policy_helpers.test.fish); this function owns only the actual
# forks (a fresh `date` only when the breaker might be open, or after a
# failed ping) and the global state (see #464 -- a breaker that latches open
# forever silently disables registration, and therefore all live updates,
# for the life of the shell; the gate/record-failure helpers' own tests cover
# that contract directly now).
function __gpy_agent_available
    if test "$GPY_AGENT_ENABLED" = 0
        return 1
    end

    if not __gpy_resolve_agent_binary >/dev/null
        return 1
    end

    # Circuit breaker: skip date fork entirely when backoff is not active (common case)
    if test "$__gpy_agent_backoff_until" -gt 0
        # Take a FRESH timestamp here rather than reusing the per-render
        # `__gpy_prompt_now` memo. That memo is only refreshed by fish_prompt, so
        # an idle shell -- precisely the shell the restart-recovery path exists
        # for -- carries an hours-stale value that makes a long-elapsed backoff
        # look permanently active. The fork only happens while the breaker is
        # actually open (the degraded path), so the hot path stays fork-free via
        # the `-gt 0` check above.
        set -l _now_check (date +%s 2>/dev/null)
        if test (__gpy_breaker_gate "$__gpy_agent_backoff_until" "$_now_check") = deny
            return 1
        end
        # Backoff elapsed (or now unknown): close the breaker and fall through
        # to a real ping.
        set -g __gpy_agent_backoff_until 0
    end

    # Try to ping the agent
    # IMPORTANT: Check if result is non-empty, not just exit code
    # Empty string still returns exit code 0, causing false positives
    set -l ping_result (__gpy_request ping 2>/dev/null)
    if test -n "$ping_result"
        # Success - reset failure count
        set -g __gpy_agent_failure_count 0
        return 0
    end

    # Failed. Only fork a fresh timestamp when this failure would open the
    # breaker (reaching the threshold) -- same fork profile as today, where
    # the plain increment case never forks. `__gpy_breaker_record_failure`
    # owns the actual threshold comparison; this is a cheap arithmetic
    # peek to decide whether the fork is needed, not a second copy of the
    # breaker's logic.
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
