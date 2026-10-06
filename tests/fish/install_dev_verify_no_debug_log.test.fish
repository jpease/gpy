#!/usr/bin/env fish
# tests/fish/install_dev_verify_no_debug_log.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #743: install-dev.fish's _verify_installation probed
# config hot-reload by starting the user's long-lived agent with
# GPY_DEBUG_LOG=/tmp/gpy-verify-<pid>.log, so that daemon traced into a
# predictable /tmp file forever (rm -f cannot stop it; the agent reopens the
# path on every line). The probe must use a private temp dir and the agent
# left running afterwards must start without GPY_DEBUG_LOG.
#
# A stub gpy-agent records "<subcommand> GPY_DEBUG_LOG=<value|<unset>>" per
# call. No real agent is started or stopped.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -g failed 0

function __gpy_test_fail --argument-names msg
    echo "❌ $msg"
    set -g failed 1
end

function __gpy_test_pass --argument-names msg
    echo "✅ $msg"
end

set -l home_dir (mktemp -d)
set -l stub_dir (mktemp -d)
set -l fish_dir (path dirname (command -v fish))
set -l safe_path /usr/bin:/bin:$fish_dir
set -l calls "$stub_dir/calls"
set -l marker "$home_dir/marker"
touch "$calls"

printf '%s\n' \
    '#!/bin/sh' \
    'echo "$1 GPY_DEBUG_LOG=${GPY_DEBUG_LOG:-<unset>}" >> "'"$calls"'"' \
    'case "$1" in' \
    '--version) echo "gpy-agent 0.0.0" ;;' \
    'start) [ -n "$GPY_DEBUG_LOG" ] && echo "Config hot-reload enabled" >> "$GPY_DEBUG_LOG" ;;' \
    esac \
    'exit 0' >"$stub_dir/gpy-agent"
chmod +x "$stub_dir/gpy-agent"

mkdir -p "$home_dir/.config/fish/gpy/core"
set -l driver "$home_dir/driver.fish"
printf '%s\n' \
    "cd '$repo_root'" \
    "set -gx PATH '$safe_path'" \
    "source install-dev.fish --fish-only >/dev/null 2>&1" \
    "set -gx PATH '$stub_dir' '$safe_path'" \
    _verify_installation >$driver

touch "$marker"
sleep 1
set -l out (env -i HOME=$home_dir XDG_CONFIG_HOME="$home_dir/.config" XDG_CACHE_HOME="$home_dir/.cache" \
    PATH="$stub_dir:$safe_path" fish --no-config $driver 2>&1)

set -l last_start (string match -e 'start ' <$calls)[-1]
if test "$last_start" = "start GPY_DEBUG_LOG=<unset>"
    __gpy_test_pass "the last agent start carries no GPY_DEBUG_LOG"
else
    __gpy_test_fail "last start was '$last_start' (calls: "(string join '; ' <$calls)")"
end

if string match -q '*Config hot-reload: ENABLED*' -- $out
    __gpy_test_pass "hot-reload verification line still prints"
else
    __gpy_test_fail "no 'Config hot-reload: ENABLED' in: $out"
end

set -l stale (find /tmp -maxdepth 1 -name 'gpy-verify-*' -newer "$marker" 2>/dev/null)
if test -z "$stale"
    __gpy_test_pass "no /tmp/gpy-verify-* file created"
else
    __gpy_test_fail "stray probe file: $stale"
    rm -f $stale
end

rm -rf "$home_dir" "$stub_dir"

if test $failed -eq 1
    exit 1
end

echo "✅ install-dev.fish's _verify_installation leaves no GPY_DEBUG_LOG on the running agent"
