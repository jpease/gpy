# tests/lib/supervisor_off.bash
#
# Sourced by bash tests that must neither start nor supervise an agent; the
# Bash twin of tests/lib/supervisor_off.zsh. gpy.bash makes its startup
# supervisor decision from the environment, but the theme export it then
# loads re-exports GPY_AGENT_SUPERVISOR_ENABLED from config.toml for every
# later prompt's check, so it takes both to turn supervision off (#657, #762,
# #835, #836).
#
# `source supervisor_off.bash DIR` points XDG_CONFIG_HOME and XDG_CACHE_HOME
# at DIR/config and DIR/cache, with a DIR/config/gpy/config.toml that
# disables the supervisor. DIR is the test's own sandbox: the test removes it
# (bash has a single EXIT trap, which the test owns) and keeps its missing
# agent socket there too, never in the checkout. Source it before any
# test-specific XDG_* exports so those still win.

if [[ -z "${1:-}" || ! -d "$1" ]]; then
    echo "supervisor_off.bash: expected an existing sandbox directory, got '${1:-}'" >&2
    exit 1
fi
mkdir -p "$1/config/gpy" "$1/cache"
printf '[agent.supervisor]\nenabled = false\n' >"$1/config/gpy/config.toml"
export XDG_CONFIG_HOME="$1/config"
export XDG_CACHE_HOME="$1/cache"
export GPY_AGENT_SUPERVISOR_ENABLED=0
