#!/usr/bin/env fish
# tests/fish/segment_position_mixed_renderers.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #765.
#
# A shell-rendered segment (status, clock, ...) that follows an agent-rendered
# one was drawn as the first segment: the shell-side renderer reads
# `__gpy_segment_position`, which fish_prompt reset once per render and only
# the shell-side helpers ever flipped. fish_prompt's dispatch loop must set the
# global from the same index that produces `is_first`.
#
# Drives the real fish_prompt dispatch loop with a stub agent-rendered segment
# (never touches `__gpy_segment_position`) and the real status segment, with
# plain-ASCII delimiters.

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l saved_home $HOME
set -l saved_xdg_config $XDG_CONFIG_HOME
set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
mkdir -p $XDG_CONFIG_HOME/gpy

# Supervisor disabled: full init path without spawning a daemon.
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
set -gx GPY_TEST_SEGMENTS "agentx status"

function segment_agentx_detect
    return 0
end
function segment_agentx_render --argument-names is_last is_first
    printf AGENT
end

source fish/core/init.fish
source fish/functions/fish_prompt.fish

set -g __segment_delim_first F
set -g __segment_delim_start S
set -g __segment_delim_end E
set -g __segment_delim_last L
set -g __icon_status_ok OK
set -g __gpy_last_status 0
set -g __gpy_add_newline 0
set -g __gpy_is_root 1

# Run fish_prompt with a clean exit status (it reads $status on entry) and
# return its output with ANSI escapes stripped.
function render_plain
    true
    fish_prompt | string collect | string replace -ra '\e\[[0-9;]*m' ''
end

set -g __enabled_segments agentx status
set -l out (render_plain | string collect)
check "agent segment then status: status opens with delim_start" (string match -q '*AGENTSOK*' -- $out; and echo pass; or echo fail)
check "agent segment then status: no first-delimiter before status" (string match -q '*AGENTFOK*' -- $out; and echo fail; or echo pass)
if not string match -q '*AGENTSOK*' -- $out
    echo "  got: "(string escape -- $out)
end

set -g __enabled_segments status agentx
set out (render_plain | string collect)
check "status then agent segment: status opens with delim_first" (string match -q 'FOK*' -- $out; and echo pass; or echo fail)
if not string match -q 'FOK*' -- $out
    echo "  got: "(string escape -- $out)
end

rm -rf $temp_home
set -gx HOME $saved_home
if test -n "$saved_xdg_config"
    set -gx XDG_CONFIG_HOME $saved_xdg_config
else
    set -e XDG_CONFIG_HOME
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count mixed-renderer segment position tests passed"
