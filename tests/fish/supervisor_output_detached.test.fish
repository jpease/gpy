#!/usr/bin/env fish
# tests/fish/supervisor_output_detached.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Fish agent supervisor must not keep the spawning shell's stdout/stderr:
# with GPY_VERBOSE=1 and an unhealthy agent, nothing it logs may reach the
# spawning shell's output (#767).

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish

set -l tmp (mktemp -d)
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx XDG_CONFIG_HOME $tmp/config
set -gx XDG_CACHE_HOME $tmp/cache
mkdir -p $XDG_RUNTIME_DIR $XDG_CONFIG_HOME/fish $XDG_CACHE_HOME
ln -s $repo_root/fish $XDG_CONFIG_HOME/fish/gpy
set -g pidfile $XDG_RUNTIME_DIR/gpy/supervisor.pid

function cleanup --inherit-variable tmp
    if test -f $pidfile
        set -l pid (cat $pidfile 2>/dev/null)
        test -n "$pid"; and kill $pid 2>/dev/null
    end
    rm -rf $tmp
end

env GPY_VERBOSE=1 GPY_AGENT_BINARY_PATH=/usr/bin/true \
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=1 \
    fish --no-config -c '
        source $argv[1]/fish/core/constants.fish
        source $argv[1]/fish/core/util.fish
        source $argv[1]/fish/core/ipc.fish
        __gpy_agent_supervisor_start' -- $repo_root >$tmp/out 2>&1

if not poll_until 3 test -s $pidfile
    echo "❌ Supervisor never wrote $pidfile"
    cleanup
    exit 1
end
sleep 2

if test -s $tmp/out
    echo "❌ Supervisor output reached the spawning shell:"
    cat $tmp/out
    cleanup
    exit 1
end

cleanup
echo "✅ Supervisor output is detached from the spawning shell"
