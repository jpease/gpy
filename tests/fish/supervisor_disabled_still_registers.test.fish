#!/usr/bin/env fish
# tests/fish/supervisor_disabled_still_registers.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With the supervisor disabled ([agent.supervisor] enabled = false) and the
# agent running, the first prompt of a Fish shell in a plain directory still
# registers it with the agent; only the per-prompt restart check is skipped.

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

init_test_env; or exit 1
mkdir -p $XDG_CONFIG_HOME/gpy
printf '%s\n' '[agent.supervisor]' 'enabled = false' >$XDG_CONFIG_HOME/gpy/config.toml

if not start_test_agent
    fail "test agent did not start"
    exit 1
end

set -l plain $__gpy_test_tmp_dir/plain
mkdir -p $plain
set -l ready $__gpy_test_tmp_dir/ready
set -l go $__gpy_test_tmp_dir/go
# The shell stays alive (registered) until the test lets it go.
fish --no-config -c '
    source $argv[1]/fish/functions/fish_prompt.fish
    cd $argv[2]
    fish_prompt >/dev/null 2>&1
    touch $argv[3]
    for i in (seq 100)
        test -e $argv[4]; and break
        sleep 0.1
    end' -- $repo_root $plain $ready $go &
set -l shell_pid $last_pid
test_track_pid $shell_pid

poll_until 5 test -e $ready; or fail "the shell drew no prompt"
if poll_until 3 __gpy_agent_has_registered_client
    pass "the first prompt registers with the supervisor disabled"
else
    fail "the first prompt registers with the supervisor disabled" (gpy-agent status 2>&1 | string collect)
end
touch $go
wait $shell_pid

test_finish "Shell registers with the supervisor disabled"
