#!/usr/bin/env bash
# tests/bash/debug_trap.test.bash
#
# Regression test for #320: the DEBUG trap gpy installs for preexec emulation
# (a) referenced $COMP_LINE/$PROMPT_COMMAND without a default, which errors
#     under `set -u` on every single command, and
# (b) overwrote any DEBUG trap already installed (e.g. bash-preexec, a user
#     framework) instead of chaining it.
#
# This sources gpy.bash under `set -u`, runs a command through the DEBUG
# trap, and asserts both that no unbound-variable error is produced and that
# a pre-existing DEBUG trap still fires.

# shellcheck disable=SC2016
# The single quotes are load-bearing throughout this file: the assertions grep
# for literal `$VAR` text in a script under test, so expanding here would test
# the wrong thing.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

export GPY_AGENT_SUPERVISOR_ENABLED=0
# Hermetic XDG dirs: sourcing gpy.bash evals the theme export, which sets
# GPY_AGENT_SUPERVISOR_ENABLED from config and overrides the export above, so
# the sandbox config disables the supervisor too. Otherwise a prompt's
# supervisor check starts a real agent on the "missing" socket, which also
# lives in the sandbox rather than the checkout (#835).
__gpy_debug_trap_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-dt-xdg.XXXXXX")
export XDG_CACHE_HOME="$__gpy_debug_trap_xdg_root/cache"
export XDG_CONFIG_HOME="$__gpy_debug_trap_xdg_root/config"
mkdir -p "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME/gpy"
printf '[agent.supervisor]\nenabled = false\n' >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_SOCKET_PATH="$__gpy_debug_trap_xdg_root/missing.sock"

echo "=== Testing DEBUG trap chains a pre-existing trap ==="
__gpy_prior_trap_fired=0
__gpy_prior_trap() { __gpy_prior_trap_fired=1; }
trap '__gpy_prior_trap' DEBUG

source bash/gpy.bash
# shellcheck disable=SC2154
# (__gpy_debug_oneshot and __gpy_prev_debug_trap are set by gpy.bash above.)

if (( BASH_VERSINFO[0] < 4 )); then
    echo "SKIP: bash $BASH_VERSION installs no gpy DEBUG trap; nothing to chain"
else
    # gpy installs its trap from the first PROMPT_COMMAND; run that one-shot
    # entry here, at this script's top level, the way bash would.
    eval "$__gpy_debug_oneshot"
    # The prior trap also fired on the `source` line above; reset the flag so
    # only a firing through gpy's chain can set it (#683).
    __gpy_prior_trap_fired=0
    if [[ "$__gpy_prev_debug_trap" != "__gpy_prior_trap" ]]; then
        echo "FAIL: captured DEBUG body is [$__gpy_prev_debug_trap], want [__gpy_prior_trap]"
        exit 1
    fi
    BASH_COMMAND=':'
    __gpy_debug_trap
    if [[ "$__gpy_prior_trap_fired" -ne 1 ]]; then
        echo "FAIL: pre-existing DEBUG trap did not fire after gpy init"
        exit 1
    fi
    echo "PASS: pre-existing DEBUG trap still fires after gpy init"

    captured="$__gpy_prev_debug_trap"
    source bash/gpy.bash
    eval "$__gpy_debug_oneshot"
    if [[ "$__gpy_prev_debug_trap" != "$captured" ]]; then
        echo "FAIL: re-sourcing changed the captured trap to [$__gpy_prev_debug_trap]"
        exit 1
    fi
    echo "PASS: re-sourcing gpy.bash keeps the captured DEBUG trap"
fi

echo "=== Testing EXIT trap containing quotes chains ==="
for gpy_bash_bin in /bin/bash "$BASH"; do
    out=$("$gpy_bash_bin" -c 'trap "echo '"'"'bye'"'"'" EXIT; source "$1/bash/gpy.bash"; exit' _ "$ROOT" 2>&1)
    if [[ "$out" != *bye* || "$out" == *"unexpected EOF"* ]]; then
        echo "FAIL: EXIT trap with quotes under $gpy_bash_bin: $out"
        exit 1
    fi
done
echo "PASS: EXIT trap containing quotes runs at exit"

echo "=== Testing DEBUG trap is safe under set -u ==="
OUT_LOG="$(mktemp)"
cleanup() { rm -f "$OUT_LOG"; }
trap cleanup EXIT

# Scrub every GPY_* variable inherited from the developer's shell. A real gpy
# install exports GPY_AGENT_ENABLED, which would satisfy `set -u` here and hide
# exactly the unbound-variable bug this test exists to catch — the test passed
# locally while CI, with a clean environment, failed (#270).
gpy_scrub=()
while IFS='=' read -r __gpy_env_name _; do
    case "$__gpy_env_name" in
        GPY_*) gpy_scrub+=(-u "$__gpy_env_name") ;;
    esac
done < <(env)

env "${gpy_scrub[@]}" bash -c '
    set -u
    ROOT="'"$ROOT"'"
    cd "$ROOT" || exit 1
    export GPY_AGENT_SUPERVISOR_ENABLED=0
    export GPY_AGENT_SOCKET_PATH="'"$__gpy_debug_trap_xdg_root"'/missing.sock"
    source bash/gpy.bash
    unset COMP_LINE
    BASH_COMMAND=":"
    __gpy_debug_trap
' >"$OUT_LOG" 2>&1
status=$?

if [[ $status -ne 0 ]]; then
    echo "FAIL: DEBUG trap errored under set -u (exit $status):"
    cat "$OUT_LOG"
    exit 1
fi
if grep -qi "unbound variable" "$OUT_LOG"; then
    echo "FAIL: DEBUG trap referenced an unset variable under set -u:"
    cat "$OUT_LOG"
    exit 1
fi
echo "PASS: DEBUG trap produces zero errors under set -u"

rm -rf "$__gpy_debug_trap_xdg_root"
echo "=== All Tests Passed ==="
