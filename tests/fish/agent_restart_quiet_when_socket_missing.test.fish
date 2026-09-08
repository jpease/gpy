#!/usr/bin/env fish
# tests/fish/agent_restart_quiet_when_socket_missing.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Verifies that cold-start recovery does not call `gpy-agent stop` when the
# socket is absent, which would otherwise print a noisy stdout notice.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l temp_dir (mktemp -d)
set -l fake_agent "$temp_dir/fake-gpy-agent.sh"
set -l stop_marker "$temp_dir/stop-called"
set -l start_marker "$temp_dir/start-called"

printf '%s\n' \
    '#!/bin/sh' \
    'case "$1" in' \
    "  stop) echo stop-called; : > \"$stop_marker\" ;;" \
    "  start) : > \"$start_marker\" ;;" \
    'esac' >"$fake_agent"
chmod +x "$fake_agent"

set -gx XDG_RUNTIME_DIR "$temp_dir/runtime"
mkdir -p "$XDG_RUNTIME_DIR/gpy"

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME "$temp_dir/cache"

source fish/core/init.fish

functions -e __gpy_resolve_agent_binary 2>/dev/null
function __gpy_resolve_agent_binary --inherit-variable fake_agent
    echo "$fake_agent"
end

functions -e __gpy_wait_for_socket 2>/dev/null
function __gpy_wait_for_socket
    return 1
end

set -l output (__gpy_agent_restart 2>&1)

if test -n "$output"
    echo "❌ Expected no output when socket is missing, got: $output"
    rm -rf "$temp_dir"
    exit 1
end

if test -f "$stop_marker"
    echo "❌ Expected cold-start restart to skip 'gpy-agent stop' when socket is missing"
    rm -rf "$temp_dir"
    exit 1
end

if not test -f "$start_marker"
    echo "❌ Expected restart helper to invoke agent start"
    rm -rf "$temp_dir"
    exit 1
end

rm -rf "$temp_dir"
echo "✅ Cold-start agent restart stays quiet when the socket is missing"
