#!/usr/bin/env zsh

# tests/zsh/basic.test.zsh

# Get project root
ROOT=${0:a:h:h:h}
cd "$ROOT"

# Sourcing the entry point starts a supervised gpy-agent (#750). Keep it inside
# a throwaway XDG root and stop it on exit so a run leaves no daemon behind,
# whether run directly or under quality-check.sh.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
basic_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-basic-xdg.XXXXXX")"
mkdir -p "$basic_xdg_root/cache" "$basic_xdg_root/config"
export XDG_CACHE_HOME="$basic_xdg_root/cache" XDG_CONFIG_HOME="$basic_xdg_root/config"
unset XDG_RUNTIME_DIR GPY_AGENT_SOCKET_PATH
trap 'shell_e2e_stop_agent_under "$basic_xdg_root/cache" "$basic_xdg_root/config"; rm -rf "$basic_xdg_root"' EXIT

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
# Exercise the pure-zsh fallback specifically. The clock is agent-rendered
# whenever the theme sets [segments.clock].format and the daemon answers, so
# without dropping the request helper this would assert against agent ANSI and
# never reach the transparent-color mapping below.
(( $+functions[__gpy_request_clock] )) && unfunction __gpy_request_clock
__color_clock_bg="transparent" __color_clock_fg="white"
clock_out=$(__gpy_segment_clock)
if [[ "$clock_out" != *"%K{default}"* ]]; then
    echo "FAIL: transparent clock bg should map to %K{default}; got: $clock_out"
    exit 1
fi

echo "PASS"
