#!/usr/bin/env fish
# tests/fish/supervisor_cadence.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Verifies that supervisor timing values exported by the agent govern the Fish
# supervisor loop (check interval, restart attempts, long backoff).

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

source fish/core/init.fish
source fish/core/ipc.fish

# Stub agent restart to fail so we exercise retry logic
functions -e __gpy_agent_restart 2>/dev/null
function __gpy_agent_restart --inherit-variable __restart_calls
    set -g __restart_calls (math "$__restart_calls + 1")
    return 1
end

# Force the supervisor to see the agent as unhealthy
functions -e __gpy_agent_is_healthy 2>/dev/null
function __gpy_agent_is_healthy
    return 1
end

# Silence log output during the test
for __fn in __gpy_log_warn __gpy_log_info __gpy_log_error
    functions -e $__fn 2>/dev/null
    function $__fn; end
end

# Provide a dummy gpy-agent command so command -q succeeds
functions -e gpy-agent 2>/dev/null
function gpy-agent
    return 0
end

# Deterministic "current time" so restart rate limiting behaves predictably
set -gx __supervisor_fake_time 100
functions -e date 2>/dev/null
function date
    if test (count $argv) -eq 1 -a "$argv[1]" = "+%s"
        set -l current $__supervisor_fake_time
        set -gx __supervisor_fake_time (math "$__supervisor_fake_time + 10")
        echo $current
        return 0
    end
    command date $argv
end

# Capture sleep invocations without introducing delays
set -gx __supervisor_sleep_calls
functions -e sleep 2>/dev/null
function sleep
    set -ga __supervisor_sleep_calls $argv[1]
    if test $argv[1] = "$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS"
        # Once long backoff is observed, disable supervisor to exit loop
        set -gx GPY_AGENT_SUPERVISOR_ENABLED "0"
    end
end

# Supervisor timing exported by the agent/theme
set -gx GPY_AGENT_SUPERVISOR_ENABLED "1"
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 7
set -gx GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS 2
set -gx GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS 5
set -gx GPY_SUPERVISOR_LONG_BACKOFF_SECONDS "1"23

set -g __restart_calls 0
set -g __supervisor_sleep_calls

__gpy_agent_supervisor_loop

if test $__restart_calls -ne 2
    echo "❌ Expected supervisor to attempt 2 restarts, got $__restart_calls"
    exit 1
end

if not contains -- $GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS $__supervisor_sleep_calls
    echo "❌ Supervisor failed to respect GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS (sleep calls: $__supervisor_sleep_calls)"
    exit 1
end

if not contains -- $GPY_SUPERVISOR_LONG_BACKOFF_SECONDS $__supervisor_sleep_calls
    echo "❌ Supervisor failed to respect GPY_SUPERVISOR_LONG_BACKOFF_SECONDS=$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS (sleep calls: $__supervisor_sleep_calls)"
    exit 1
end

echo "✅ Supervisor honours restart cadence derived from config"
