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
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing.sock"

echo "=== Testing DEBUG trap chains a pre-existing trap ==="
__gpy_prior_trap_fired=0
__gpy_prior_trap() { __gpy_prior_trap_fired=1; }
trap '__gpy_prior_trap' DEBUG

source bash/gpy.bash

# Invoke the installed DEBUG trap the way bash would for a real command.
BASH_COMMAND=':'
__gpy_debug_trap

if [[ "$__gpy_prior_trap_fired" -ne 1 ]]; then
    echo "FAIL: pre-existing DEBUG trap did not fire after gpy init"
    exit 1
fi
echo "PASS: pre-existing DEBUG trap still fires after gpy init"

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
    export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing.sock"
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

echo "=== All Tests Passed ==="
