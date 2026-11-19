#!/usr/bin/env fish
# tests/fish/first_prompt_agent_start.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fish's prompt is drawn by `gpy prompt fish`, which also keeps the shell's
# lifecycle: the first prompt runs `gpy-agent start` exactly once even when
# the agent already answers (`start` keeps or replaces the agent itself), a
# later prompt never starts it again while it answers, an unreachable agent is
# restarted by later prompts up to the restart attempt limit, and an agent
# disabled in the environment is never started.
#
# A copy of this checkout's `gpy` sits beside a stand-in `gpy-agent` that only
# logs its arguments (`gpy` prefers the `gpy-agent` installed beside it), and
# each shell is a `fish -c` that loads fish/functions/fish_prompt.fish and
# draws its prompts.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"

print_test_header "The first prompt starts the agent once"

if not command -q python3
    test_skip "python3 not installed, cannot simulate an agent"
end

set -l target_dir $repo_root/gpy-agent/target
test -n "$CARGO_TARGET_DIR"; and set target_dir $CARGO_TARGET_DIR
if not test -x $target_dir/debug/gpy
    fail "gpy not built (cd gpy-agent && cargo build --bins)"
    exit 1
end

set -g tmp (mktemp -d)
test_track_dir $tmp
set -gx XDG_CACHE_HOME $tmp/cache
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
mkdir -p $tmp/bin $XDG_CACHE_HOME $XDG_CONFIG_HOME $XDG_RUNTIME_DIR
cp $target_dir/debug/gpy $tmp/bin/gpy
set -g calls $tmp/agent.calls
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$calls'" 'exit 0' >$tmp/bin/gpy-agent
chmod +x $tmp/bin/gpy-agent
set -gx PATH $tmp/bin $PATH

function __starts
    test -f $calls; or begin
        echo 0
        return
    end
    count (string match -r '^start$' <$calls)
end

function __starts_at_least --argument-names minimum
    test (__starts) -ge $minimum
end

# One new shell drawing $argv[1] prompts.
function __shell_prompts --argument-names n
    fish --no-config -c '
        source $argv[1]/fish/functions/fish_prompt.fish
        for i in (seq $argv[2])
            fish_prompt >/dev/null 2>&1
        end' -- $repo_root $n
end

function __reset
    rm -f $calls
    set -e GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS
    set -e GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS
    set -gx GPY_AGENT_ENABLED 1
end

# --- a healthy agent is started once per shell, never again ---

__reset
test_stub_listener $tmp/agent.sock reply --lifetime 30 --reply '{"status":"ok"}'
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
__shell_prompts 3
poll_until 5 __starts_at_least 1; or fail "a healthy agent did not make the first prompt run start"
# Absence cannot be polled for: give a wrongly started process a fixed window.
sleep 0.5
check "a healthy agent gets exactly one start per shell" 1 (__starts)
test_stop_pids $test_stub_pid
rm -f $tmp/agent.sock

# --- an unreachable agent: the startup start plus capped restarts ---

__reset
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
set -gx GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS 2
__shell_prompts 7
poll_until 5 __starts_at_least 3; or fail "an unreachable agent was not restarted up to the attempt cap"
sleep 0.5
check "an unreachable agent gets the startup start plus the capped restarts" 3 (__starts)

# --- a second shell starts it again on its own first prompt ---

__reset
__shell_prompts 1
poll_until 5 __starts_at_least 1
check "a new shell's first prompt runs start" 1 (__starts)

# --- the agent disabled: no prompt starts it ---

__reset
set -gx GPY_AGENT_ENABLED 0
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
__shell_prompts 3
sleep 0.5
check "a disabled agent is never started" 0 (__starts)

test_finish "The first prompt starts the agent once"
