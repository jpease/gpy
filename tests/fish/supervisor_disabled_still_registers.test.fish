#!/usr/bin/env fish
# tests/fish/supervisor_disabled_still_registers.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With the supervisor disabled ([agent.supervisor] enabled = false, exported
# as GPY_AGENT_SUPERVISOR_ENABLED=0) and the agent running, the first prompt
# in a plain directory still registers the shell with the agent; only the
# background restart loop is skipped (#700).

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -gx PATH $repo_root/gpy-agent/target/debug $PATH
if not command -q gpy-agent
    echo "❌ gpy-agent not built (cd gpy-agent && cargo build)"
    exit 1
end

init_test_env; or exit 1
set -gx XDG_RUNTIME_DIR $__gpy_test_tmp_dir/run
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/gpy $XDG_CACHE_HOME/gpy
printf '%s\n' '[agent.supervisor]' 'enabled = false' >$XDG_CONFIG_HOME/gpy/config.toml
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid

function finish --argument-names code
    if test -f $pidfile
        set -l pid (cat $pidfile 2>/dev/null)
        test -n "$pid"; and kill $pid 2>/dev/null
    end
    stop_test_agent
    cleanup_test_files
    exit $code
end

function fail
    echo "❌ $argv"
    finish 1
end

start_test_agent; or fail "test agent did not start"

set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
set -gx GPY_AGENT_ENABLED 1
printf '%s\n' 'set -gx GPY_AGENT_ENABLED "1"' 'set -gx GPY_AGENT_SUPERVISOR_ENABLED "0"' >$XDG_CACHE_HOME/gpy/theme-export.fish

source $repo_root/fish/core/init.fish
set -l plain (mktemp -d)
cd $plain
emit fish_prompt

set -q __gpy_registered; or fail "shell did not register on its first prompt with the supervisor disabled"
poll_until 3 __gpy_agent_has_registered_client; or fail "agent reports no registered client"
functions -q __gpy_start_supervisor_on_prompt; and fail "registration hook was not removed after registering"
test -e $pidfile; and fail "supervisor.pid was created with GPY_AGENT_SUPERVISOR_ENABLED=0"

cd /
rm -rf $plain
echo "✅ Shell registers with the supervisor disabled"
finish 0
