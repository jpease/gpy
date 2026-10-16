#!/usr/bin/env bash

# tests/bash/basic.test.bash

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
basic_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-basic-xdg.XXXXXX")"
mkdir -p "$basic_xdg_root/cache" "$basic_xdg_root/config"
export XDG_CACHE_HOME="$basic_xdg_root/cache" XDG_CONFIG_HOME="$basic_xdg_root/config"
unset XDG_RUNTIME_DIR GPY_AGENT_SOCKET_PATH
trap 'shell_e2e_stop_agent_under "$basic_xdg_root/cache" "$basic_xdg_root/config"; rm -rf "$basic_xdg_root"' EXIT

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
# Drop the request helper so the local (non-agent) clock path runs.
unset -f __gpy_request_clock 2>/dev/null || true
__clock_format=""
__color_clock_bg="transparent" __color_clock_fg="white"
clock_out="$(__gpy_segment_clock "" "" "true")"
if [[ "$clock_out" != *"37;49m"* ]]; then
    echo "FAIL: transparent clock bg should render as default (49); got: $clock_out"
    exit 1
fi

echo "PASS"
