# tests/lib/supervisor_off.zsh
#
# Sourced by zsh tests that must neither start nor supervise an agent. The
# theme export re-exports GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED
# from config.toml after gpy.zsh loads, so an environment value alone does not
# turn them off (#657, #762). `[agent.supervisor] enabled = false` alone only
# stops restarts: the startup start depends on `[agent] enabled` (#842), so the
# agent is disabled as well. A test that needs the IPC paths exports
# GPY_AGENT_ENABLED=1 after sourcing gpy.zsh. This points XDG_CONFIG_HOME and
# XDG_CACHE_HOME at a throwaway root whose config.toml disables both, and
# removes it on exit.
# Source it before any test-specific XDG_* exports so those still win.

__gpy_test_supervisor_off_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-supervisor-off.XXXXXX") || exit 1
mkdir -p "$__gpy_test_supervisor_off_root/config/gpy" "$__gpy_test_supervisor_off_root/cache"
print -r -- $'[agent]\nenabled = false\n\n[agent.supervisor]\nenabled = false' >"$__gpy_test_supervisor_off_root/config/gpy/config.toml"
export XDG_CONFIG_HOME="$__gpy_test_supervisor_off_root/config"
export XDG_CACHE_HOME="$__gpy_test_supervisor_off_root/cache"
export GPY_AGENT_ENABLED=0
export GPY_AGENT_SUPERVISOR_ENABLED=0

function __gpy_test_supervisor_off_cleanup() {
    rm -rf "$__gpy_test_supervisor_off_root"
}
zshexit_functions+=(__gpy_test_supervisor_off_cleanup)
