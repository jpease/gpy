#!/usr/bin/env fish
# tests/fish/ipc_error_reply_refusal.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The reply contract: a reply starting `{"error"` (a denied cwd, an
# unparseable request) is a permanent refusal. No requester prints it into the
# prompt or memoizes it; requesters with a oneshot fallback take it, the
# others render nothing (or their local pill). An empty reply line still means
# "alive but late": no oneshot fork.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/renderer.fish"
source "$repo_root/fish/segments/directory.fish"
source "$repo_root/fish/segments/clock.fish"

print_test_header "IPC error replies are refusals, never prompt text"

if not command -q python3
    test_skip "python3 not installed, cannot simulate an agent"
end
if not __gpy_ipc_client_available
    test_skip "neither socat nor an nc with -U is installed"
end

set -g tmp (mktemp -d)
test_track_dir $tmp
mkdir -p $tmp/proj
set -gx XDG_CACHE_HOME $tmp/cache
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Oneshot stand-in: logs its argv and prints the op name in capitals.
set -g calls $tmp/calls.log
printf '%s\n' '#!/bin/sh' "echo \"\$*\" >>'$calls'" \
    'case "$1" in oneshot) printf "%s" "$2" | tr a-z A-Z;; esac' >$tmp/gpy-agent
chmod +x $tmp/gpy-agent
set -gx PATH $tmp $PATH

set -g __gpy_theme_name test
set -g __color_directory_bg blue
set -g __color_clock_bg blue
set -g __color_clock_fg white
set -g __clock_format 1

function __oneshots --argument-names op
    test -f $calls; or begin
        echo 0
        return
    end
    count (string match -- "oneshot $op*" <$calls)
end

function __fresh_render
    set -e __gpy_oneshot_used
    set -e __gpy_dir_cache_key
    set -e __gpy_dir_cache_val
    set -e __gpy_clock_cache_key
    set -e __gpy_clock_cache_val
    set -g __gpy_last_segment_bg black
    rm -f $calls
end

# --- A refusal ---

set -gx GPY_AGENT_SOCKET_PATH $tmp/refuse.sock
set -l refusal '{"error":"Invalid path: Access to system directories denied"}'
test_stub_listener $GPY_AGENT_SOCKET_PATH reply --reply $refusal --lifetime 30

__fresh_render
set -l raw (__gpy_ipc_send '{"op":"directory","cwd":"/etc","format":"ansi"}' 1000)
check "__gpy_ipc_send returns 3 on an error reply" 3 $status
check "__gpy_ipc_send prints nothing on an error reply" "" "$raw"
check "the refusal is kept for callers that need it" $refusal "$__gpy_ipc_error_reply"

__fresh_render
set -l dir_out (segment_directory_render true true | string collect)
check "a refused directory request renders through oneshot" DIRECTORY "$dir_out"
check "the oneshot render, not the refusal, is memoized" DIRECTORY "$__gpy_dir_cache_val"

__fresh_render
set -l dur_out (__gpy_request_duration 500 true black true | string collect)
check "a refused duration request renders through oneshot" DURATION "$dur_out"

__fresh_render
set -l char_out (__gpy_request_character 1 true black | string collect)
check "a refused character request renders through oneshot" CHARACTER "$char_out"

__fresh_render
set -l status_out (__gpy_request_exit_status 1 true true black | string collect)
check "a refused exit_status request prints nothing" "" "$status_out"

__fresh_render
set -l clock_out (segment_clock_render true true | string collect)
if string match -q '*error*' -- "$clock_out"
    fail "a refused clock request never reaches the prompt: $clock_out"
else
    pass "a refused clock request never reaches the prompt"
end
check "a refused clock reply is not memoized" "" "$__gpy_clock_cache_val"

__gpy_register_with_agent
check "a refused register leaves the shell unregistered" 0 (set -q __gpy_registered; and echo 1; or echo 0)

test_stop_pids $test_stub_pid

# --- A late agent: an empty reply line forks no oneshot ---

set -gx GPY_AGENT_SOCKET_PATH $tmp/late.sock
test_stub_listener $GPY_AGENT_SOCKET_PATH reply --reply '' --lifetime 30

__fresh_render
set -l late_out (__gpy_request directory $tmp/proj true black true | string collect)
check "an empty reply prints nothing" "" "$late_out"
check "an empty reply forks no oneshot" 0 (__oneshots directory)
set -l late_raw (__gpy_ipc_send '{"op":"ping"}' 1000)
check "an empty reply is send status 2" 2 $status

test_stop_pids $test_stub_pid

test_finish "IPC error replies are refusals, never prompt text"
