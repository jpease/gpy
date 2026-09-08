#!/usr/bin/env zsh

# tests/zsh/basic.test.zsh

# Get project root
ROOT=${0:a:h:h:h}
cd "$ROOT"

echo "Sourcing gpy.zsh from $ROOT/zsh/gpy.zsh"
source zsh/gpy.zsh

echo "Checking constants..."
if [[ -z "$__gpy_ui_prompt_icon" ]]; then
    echo "FAIL: Constants not loaded"
    exit 1
fi

echo "Checking IPC..."
if ! functions __gpy_request >/dev/null; then
    echo "FAIL: IPC function not defined"
    exit 1
fi

echo "Checking init..."
if ! functions __gpy_precmd >/dev/null; then
    echo "FAIL: Init function not defined"
    exit 1
fi

echo "Checking render..."
# Mock __enabled_segments to avoid calling git/ipc in test without agent
__enabled_segments=(directory)
PROMPT=$(__gpy_render_prompt)
echo "Prompt rendered: $PROMPT"

if [[ -z "$PROMPT" ]]; then
    echo "FAIL: Prompt is empty"
    exit 1
fi

echo "Checking transparent clock background..."
# Flat themes (e.g. the Starship preset) export a transparent clock bg. It must
# map to zsh's `default` color keyword, not an unrecognized `%K{transparent}`.
source zsh/segments/clock.zsh
__color_clock_bg="transparent" __color_clock_fg="white"
clock_out=$(__gpy_segment_clock)
if [[ "$clock_out" != *"%K{default}"* ]]; then
    echo "FAIL: transparent clock bg should map to %K{default}; got: $clock_out"
    exit 1
fi

echo "PASS"
