#!/usr/bin/env bash

# tests/bash/basic.test.bash

# Get project root
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

echo "Sourcing gpy.bash from $ROOT/bash/gpy.bash"
source bash/gpy.bash

echo "Checking constants..."
if [[ -z "$__prompt_color" ]]; then
    echo "FAIL: Constants not loaded"
    exit 1
fi

echo "Checking IPC..."
if ! declare -f __gpy_request &>/dev/null; then
    echo "FAIL: IPC function not defined"
    exit 1
fi

echo "Checking init..."
if ! declare -f __gpy_precmd &>/dev/null; then
    echo "FAIL: Init function not defined"
    exit 1
fi

echo "Checking version detection..."
echo "Bash version: ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"
echo "Duration method: $__gpy_duration_method"

if [[ -z "$__gpy_duration_method" ]]; then
    echo "FAIL: Duration method not set"
    exit 1
fi

echo "Checking render..."
# Mock __enabled_segments to avoid calling git/ipc in test without agent
__enabled_segments="directory"
__gpy_render_prompt 0
echo "Prompt rendered: $PS1"

if [[ -z "$PS1" ]]; then
    echo "FAIL: PS1 is empty"
    exit 1
fi

echo "Checking transparent clock background..."
# Flat themes (e.g. the Starship preset) export a transparent clock bg. It must
# map to the terminal default (SGR 49) instead of the cyan `*)` fallback.
source bash/segments/clock.bash
# Exercise the pure-bash fallback specifically. The clock is agent-rendered
# whenever the theme sets [segments.clock].format and the daemon answers, so
# without dropping the request helper this would assert against agent ANSI and
# never reach the transparent-color mapping below.
unset -f __gpy_request_clock 2>/dev/null || true
__color_clock_bg="transparent" __color_clock_fg="white"
clock_out="$(__gpy_segment_clock)"
if [[ "$clock_out" != *"49;37m"* ]]; then
    echo "FAIL: transparent clock bg should render as default (49); got: $clock_out"
    exit 1
fi

echo "PASS"
