#!/usr/bin/env fish
# tests/fish/agent_disabled_no_supervisor.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With the agent disabled in config.toml (exported GPY_AGENT_ENABLED=0), no
# prompt may start the agent, spawn a background process, or run the
# per-prompt check, whether or not restarts are enabled.

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -l tmp (mktemp -d)
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_DATA_HOME $tmp/data
set -gx XDG_CACHE_HOME $tmp/cache
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/fish $XDG_CACHE_HOME/gpy
test_track_dir $tmp
set -g stub_log $tmp/agent-calls.log

printf '%s\n' 'set -g __gpy_export_version "1"' 'set -g __gpy_cfg_agent_enabled "0"' 'set -g __gpy_cfg_supervisor_enabled "1"' >$XDG_CACHE_HOME/gpy/theme-export.fish

printf '%s\n' '#!/bin/sh' "echo \"\$*\" >> '$stub_log'" 'exit 0' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent
set -gx PATH $tmp $PATH

# The flags under test come from the cache file alone; an exported user value
# would win over the config mirror (and is not what this test is about).
set -e GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS

function die
    fail "$argv"
    exit 1
end

source $repo_root/fish/core/init.fish
test "$__gpy_cfg_agent_enabled" = 0; or die "theme export did not set __gpy_cfg_agent_enabled=0 (got '$__gpy_cfg_agent_enabled')"
set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0

for prompt in 1 2 3
    emit fish_prompt
end
# Absence cannot be polled for: give any (wrongly) started process a fixed
# window to show up before asserting that none did.
sleep 1

test -e $XDG_RUNTIME_DIR/gpy/supervisor.pid; and die "supervisor.pid was created with GPY_AGENT_ENABLED=0"
if test -f $stub_log
    die "gpy-agent was called with GPY_AGENT_ENABLED=0: "(string join ';' <$stub_log)
end
test (count (jobs -p)) -eq 0; or die "a background job exists with GPY_AGENT_ENABLED=0"
test "$__gpy_supervisor_check_attempts" = 0; or die "the per-prompt check counted a restart attempt"

pass "no start, no background process and no restart attempt with GPY_AGENT_ENABLED=0"

test_finish "Agent-disabled shells start nothing and run no per-prompt restart"
