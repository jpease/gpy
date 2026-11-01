#!/usr/bin/env fish
# tests/fish/supervisor_disabled_still_registers.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With the supervisor disabled ([agent.supervisor] enabled = false, exported
# as GPY_AGENT_SUPERVISOR_ENABLED=0) and the agent running, the first prompt
# in a plain directory still registers the shell with the agent; only the
# per-prompt restart check is skipped.

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -gx PATH $repo_root/gpy-agent/target/debug $PATH
if not command -q gpy-agent
    fail "gpy-agent not built (cd gpy-agent && cargo build)"
    exit 1
end

init_test_env; or exit 1
set -gx XDG_RUNTIME_DIR $__gpy_test_tmp_dir/run
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/gpy $XDG_CACHE_HOME/gpy
printf '%s\n' '[agent.supervisor]' 'enabled = false' >$XDG_CONFIG_HOME/gpy/config.toml
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid

function finish --argument-names code
    stop_test_agent
    cleanup_test_files
    exit $code
end

function die
    fail "$argv"
    finish 1
end

start_test_agent; or die "test agent did not start"

printf '%s\n' 'set -g __gpy_export_version "1"' 'set -g __gpy_cfg_agent_enabled "1"' 'set -g __gpy_cfg_supervisor_enabled "0"' >$XDG_CACHE_HOME/gpy/theme-export.fish

source $repo_root/fish/core/init.fish
set -l plain (mktemp -d)
cd $plain
emit fish_prompt

set -q __gpy_registered; or die "shell did not register on its first prompt with the supervisor disabled"
poll_until 3 __gpy_agent_has_registered_client; or die "agent reports no registered client"
functions -q __gpy_supervisor_on_prompt; or die "the prompt handler must stay registered for the life of the shell"
test -e $pidfile; and die "supervisor.pid was created with GPY_AGENT_SUPERVISOR_ENABLED=0"

cd /
rm -rf $plain
pass "Shell registers with the supervisor disabled"
finish 0
