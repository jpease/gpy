#!/usr/bin/env fish
# tests/fish/prompt_autostart_backoff.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #301: the fish_prompt autostart hook must rate-limit
# and cap its restart attempts against a crash-looping agent binary instead
# of re-forking and stalling on every single prompt.

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

source fish/core/init.fish
source fish/core/ipc.fish

# Simulate a crash-looping agent: never available, and the resolved binary is
# a real no-op executable so `"$agent_binary" start &` succeeds immediately
# (crashes/exits right away, same effect as a genuinely crash-looping build).
functions -e __gpy_agent_available 2>/dev/null
function __gpy_agent_available
    return 1
end

functions -e __gpy_resolve_agent_binary 2>/dev/null
function __gpy_resolve_agent_binary
    echo /usr/bin/true
end

functions -e __gpy_register_with_agent 2>/dev/null
function __gpy_register_with_agent
    return 1
end

functions -e __gpy_agent_supervisor_start 2>/dev/null
function __gpy_agent_supervisor_start
    return 0
end

# Silence log output during the test
for __fn in __gpy_log_warn __gpy_log_info __gpy_log_debug __gpy_log_error
    functions -e $__fn 2>/dev/null
    function $__fn
    end
end

# Deterministic "current time" so the rate limit behaves predictably
set -g __autostart_fake_time 1000
functions -e date 2>/dev/null
function date
    if test (count $argv) -eq 1 -a "$argv[1]" = "+%s"
        echo $__autostart_fake_time
        return 0
    end
    command date $argv
end

# Capture sleep invocations without introducing delays
set -g __autostart_sleep_calls
functions -e sleep 2>/dev/null
function sleep
    set -ga __autostart_sleep_calls $argv[1]
end

set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_AUTOSTART_MAX_ATTEMPTS 2
set -gx GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS 5
set -gx GPY_AGENT_START_DELAY_MS 0

set -e __gpy_supervisor_initialized
set -e __gpy_autostart_attempts
set -e __gpy_autostart_last_attempt_time

# Simulate 5 prompts in a row from a crash-looping agent, with fake time
# advancing past the rate limit between each prompt so every prompt would
# otherwise be free to attempt a restart.
for __i in (seq 5)
    if functions -q __gpy_start_supervisor_on_prompt
        __gpy_start_supervisor_on_prompt
    end
    set -g __autostart_fake_time (math "$__autostart_fake_time + $GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS + 1")
end

if test (count $__autostart_sleep_calls) -ne $GPY_AGENT_AUTOSTART_MAX_ATTEMPTS
    echo "❌ Expected exactly $GPY_AGENT_AUTOSTART_MAX_ATTEMPTS autostart attempts across 5 prompts, got "(count $__autostart_sleep_calls)
    exit 1
end

if functions -q __gpy_start_supervisor_on_prompt
    echo "❌ Expected the per-prompt autostart hook to remove itself after exceeding the attempt limit"
    exit 1
end

echo "✅ Per-prompt autostart hook rate-limits and caps restarts against a crash-looping agent"
