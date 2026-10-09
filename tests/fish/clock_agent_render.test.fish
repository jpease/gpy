#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# Test: the Fish clock is agent-rendered whenever the theme sets
# `[segments.clock].format`, like Bash and Zsh (#844). Fish used to always draw
# the clock locally, so a templated theme (the default, or a custom template)
# looked different in Fish than in the other two shells.
#
# The agent's Fish response carries the clock's bare strftime spec where the
# time goes (Fish has no prompt-level time token), and segment_clock_render
# swaps the formatted time in. Stubs stand in for the request helper and the
# renderer so what is asserted is the decision and the substitution; the
# end-to-end bytes against a live agent are compared across shells in
# tests/bash/shell_contract.test.bash.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/renderer.fish"
source "$repo_root/fish/segments/clock.fish"

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

# Deterministic 12-hour clock and a known "now".
set -g __time_format 12
set -g __clock_show_leading_zero 0
set -g __clock_show_seconds 0
set -g __color_clock_bg blue
set -g __color_clock_fg white
set -g __gpy_clock_prerendered "3:07 PM"

# Record every request and answer like the agent would for a template
# `[ $time]($style)`: the spec stands where the time goes.
set -g __requests 0
function __gpy_request_clock --argument-names is_last is_first prev_bg
    set -g __requests (math $__requests + 1)
    set -g __request_args "$is_last|$is_first|$prev_bg"
    printf '%s\n' "AGENT[ "(__gpy_clock_date_format)"]"
end

# Local renderer stub: records that the shell drew the clock itself.
function gpy_section_standalone --argument-names bg fg content is_last
    set -g __local_content "$content"
end

# --- a theme with a clock format: the agent draws it, fish fills in the time ---
set -g __clock_format 1
set -g __gpy_last_segment_bg magenta
set -e __local_content
set -l out (segment_clock_render true "")
if test "$out" = "AGENT[ 3:07 PM]"
    check "templated clock: the agent's response, with the spec replaced by the time" pass
else
    check "templated clock: the agent's response, with the spec replaced by the time (got: '$out')" fail
end
if not set -q __local_content
    check "templated clock: nothing drawn locally" pass
else
    check "templated clock: nothing drawn locally (got: '$__local_content')" fail
end
if test "$__request_args" = "true||magenta"
    check "templated clock: is_last, is_first and prev_bg reach the agent in their own slots" pass
else
    check "templated clock: is_last, is_first and prev_bg reach the agent in their own slots (got: '$__request_args')" fail
end
if test "$__gpy_last_segment_bg" = blue
    check "templated clock: tracks the clock background for the next segment's chevron" pass
else
    check "templated clock: tracks the clock background for the next segment's chevron (got: '$__gpy_last_segment_bg')" fail
end

# --- the spec follows the configured time format ---
set -g __time_format 24
set -g __clock_show_leading_zero 1
set -g __gpy_clock_prerendered "15:07"
set out (segment_clock_render "" true)
if test "$out" = "AGENT[ 15:07]"
    check "templated clock: a 24-hour spec is replaced too" pass
else
    check "templated clock: a 24-hour spec is replaced too (got: '$out')" fail
end
if test "$__request_args" = "|true|blue"
    check "templated clock: a first-but-not-last clock sends is_first" pass
else
    check "templated clock: a first-but-not-last clock sends is_first (got: '$__request_args')" fail
end
set -g __time_format 12
set -g __clock_show_leading_zero 0
set -g __gpy_clock_prerendered "3:07 PM"

# --- no clock format in the theme: drawn locally, no agent round trip ---
set -g __clock_format ""
set -g __requests 0
set -e __local_content
set out (segment_clock_render true "")
if test $__requests -eq 0
    check "untemplated clock: no request is sent to the agent" pass
else
    check "untemplated clock: no request is sent to the agent (sent $__requests)" fail
end
if test "$__local_content" = " 3:07 PM"
    check "untemplated clock: drawn locally with the leading pad" pass
else
    check "untemplated clock: drawn locally with the leading pad (got: '$__local_content')" fail
end

# --- templated theme, agent unreachable: falls back to the local clock ---
set -g __clock_format 1
function __gpy_request_clock
    return 1
end
set -e __local_content
set out (segment_clock_render true "")
if test "$__local_content" = " 3:07 PM"
    check "templated clock, agent down: falls back to the local clock" pass
else
    check "templated clock, agent down: falls back to the local clock (got: '$__local_content')" fail
end

functions -e __gpy_request_clock gpy_section_standalone

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count clock agent render tests passed"
