#!/usr/bin/env fish
# tests/fish/supervisor_no_date_fork.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A prompt on which the per-prompt check is rate limited forks no `date` and
# makes no registration attempt. The retry on the prompt after the initial
# start is the only attempt between checks.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"

print_test_header "Rate-limited supervisor prompts fork no date"

set -g tmp (mktemp -d)
test_track_dir $tmp
mkdir -p $tmp/bin
# A `date` shim ahead of the real one: every call is counted.
printf '%s\n' '#!/bin/sh' "echo x >>'$tmp/date.calls'" 'exec /bin/date "$@"' >$tmp/bin/date
chmod +x $tmp/bin/date
set -gx PATH $tmp/bin $PATH

set -gx XDG_CACHE_HOME $tmp/cache
set -gx GPY_AGENT_SOCKET_PATH $tmp/absent.sock
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 100000
set -gx PATH $tmp $PATH
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$tmp/agent.calls'" 'exit 1' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent

set -g attempts 0
function __gpy_register_with_agent
    set -g attempts (math $attempts + 1)
    return 1
end

function __date_calls
    test -f $tmp/date.calls; or begin
        echo 0
        return
    end
    count <$tmp/date.calls
end

function __agent_starts
    test -f $tmp/agent.calls; or begin
        echo 0
        return
    end
    count (string match -r '^start$' <$tmp/agent.calls)
end

# First prompt: the agent is down, so it is started and a retry is pending.
__gpy_supervisor_on_prompt
check "the first prompt starts the agent once" 1 (begin
    poll_until 5 test -f $tmp/agent.calls
    __agent_starts
end)
check "the first prompt makes no registration attempt while the agent is down" 0 $attempts

# Second prompt: the one registration retry.
__gpy_supervisor_on_prompt
check "the next prompt makes one registration attempt" 1 $attempts

# The first prompt counted as the check, so every later prompt is rate limited.
set -l dates_before (__date_calls)
for i in (seq 5)
    __gpy_supervisor_on_prompt
end
check "five rate-limited prompts fork no date" $dates_before (__date_calls)
check "five rate-limited prompts make no registration attempt" 1 $attempts
check "five rate-limited prompts start nothing more" 1 (__agent_starts)

test_finish "Rate-limited supervisor prompts fork no date"
