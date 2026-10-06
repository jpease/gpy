#!/usr/bin/env bash
# tests/bash/hostile_env.test.bash
#
# Hostile-shell-environment harness. Every scenario starts a FRESH interactive
# child bash whose rc file first builds a hostile pre-existing environment
# (traps, PROMPT_COMMAND, ...), then sources gpy.bash the way the installer rc
# block does, then feeds the child a few command lines (each one a prompt
# cycle) and asserts the user's environment survived. New hostile-environment
# issues add a scenario: a `scenario_<name>` function plus one `run_scenario`
# line below.
#
# Children never start a gpy-agent: the supervisor is disabled and the socket
# path points at a file that does not exist. Paths reach the child only as
# environment/argv, never interpolated into code.

# shellcheck disable=SC2016
# Single quotes are load-bearing: the rc preludes are literal child code.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-hostile.XXXXXX")"
cleanup() { rm -rf "$SANDBOX"; }
trap cleanup EXIT
mkdir -p "$SANDBOX/cfg" "$SANDBOX/cache" "$SANDBOX/run"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }
skip() { echo "SKIP: $*"; }

# run_child <prelude> <input>: sets CHILD_OUT to the child's merged output.
# <prelude> is child rc code run before gpy.bash is sourced; the rc then
# sources gpy.bash from $GPY_TEST_ROOT at top level, like the installer's rc
# block. <input> is piped to the interactive child, one prompt cycle per line.
run_child() {
    local rc="$SANDBOX/rc.bash"
    {
        printf '%s\n' "$1"
        printf '%s\n' 'source "$GPY_TEST_ROOT/bash/gpy.bash"'
    } >"$rc"
    CHILD_OUT="$(printf '%s\n' "$2" | env -i \
        PATH=/usr/bin:/bin TERM=dumb HOME="$SANDBOX" \
        XDG_CONFIG_HOME="$SANDBOX/cfg" XDG_CACHE_HOME="$SANDBOX/cache" \
        XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" \
        "$BASH" --noprofile --rcfile "$rc" -i 2>&1)"
}

# field <name>: value printed by the child as `name=[value]`.
field() { printf '%s\n' "$CHILD_OUT" | sed -n "s/^$1=\[\(.*\)\]\$/\1/p" | sed -n 1p; }

run_scenario() {
    echo "=== Scenario: $1 ==="
    "scenario_$1"
}

scenario_prior_debug_trap_fires() {
    if ((BASH_VERSINFO[0] < 4)); then
        skip "bash ${BASH_VERSION} gets no gpy DEBUG trap; nothing to chain"
        return
    fi
    run_child '
__prior_n=0
__prior_hook() { __prior_n=$((__prior_n + 1)); }
trap "__prior_hook" DEBUG' '__prior_n=0
echo hi
echo "after=[$__prior_n]"
echo "prev=[$__gpy_prev_debug_trap]"
exit'
    [[ "$(field prev)" == "__prior_hook" ]] ||
        fail "captured DEBUG body is [$(field prev)], want [__prior_hook]"
    local after
    after="$(field after)"
    if [[ -n "$after" && "$after" -ge 1 ]]; then
        pass "pre-existing DEBUG trap still fires after gpy init"
    else
        fail "pre-existing DEBUG trap did not fire after gpy init (after=[$after])"
        printf '%s\n' "$CHILD_OUT"
    fi
}

scenario_exit_trap_with_quotes() {
    run_child 'trap "echo '"'"'bye bye'"'"'" EXIT' 'echo hi
exit'
    local n
    n="$(printf '%s\n' "$CHILD_OUT" | grep -c '^bye bye$')"
    if [[ "$CHILD_OUT" == *"unexpected EOF"* ]]; then
        fail "EXIT trap with quotes broke at exit:"
        printf '%s\n' "$CHILD_OUT"
    elif [[ "$n" -ne 1 ]]; then
        fail "EXIT trap with quotes ran $n times, want 1"
        printf '%s\n' "$CHILD_OUT"
    else
        pass "EXIT trap containing single quotes chains and runs once"
    fi
}

scenario_resource_keeps_chain() {
    run_child '
trap "__prior_hook() { :; }; echo '"'"'bye'"'"'" EXIT
__prior_hook() { :; }
trap "__prior_hook" DEBUG' 'before_exit=[$__gpy_prev_exit_trap]
echo "before_exit=[$__gpy_prev_exit_trap]"
echo "before_dbg=[$__gpy_prev_debug_trap]"
source "$GPY_TEST_ROOT/bash/gpy.bash"
echo "after_exit=[$__gpy_prev_exit_trap]"
echo "after_dbg=[$__gpy_prev_debug_trap]"
exit'
    local ok=1
    [[ -n "$(field before_exit)" && "$(field before_exit)" == "$(field after_exit)" ]] || ok=0
    [[ "$(field before_dbg)" == "$(field after_dbg)" ]] || ok=0
    if ((BASH_VERSINFO[0] >= 4)); then
        [[ -n "$(field after_dbg)" ]] || ok=0
    fi
    if ((ok)); then
        pass "re-sourcing gpy.bash keeps the captured traps"
    else
        fail "re-source changed the chain"
        printf '%s\n' "$CHILD_OUT"
    fi
}

scenario_never_chains_to_itself() {
    run_child '' 'source "$GPY_TEST_ROOT/bash/gpy.bash"
source "$GPY_TEST_ROOT/bash/gpy.bash"
echo "dbg=[$__gpy_prev_debug_trap]"
echo "exit=[$__gpy_prev_exit_trap]"
exit'
    if [[ "$(field dbg)$(field exit)" == *__gpy_* ]]; then
        fail "gpy chained to itself: dbg=[$(field dbg)] exit=[$(field exit)]"
    else
        pass "gpy never chains to itself"
    fi
}

run_scenario prior_debug_trap_fires
run_scenario exit_trap_with_quotes
run_scenario resource_keeps_chain
run_scenario never_chains_to_itself

if ((failures > 0)); then
    echo "=== $failures scenario assertion(s) failed ==="
    exit 1
fi
echo "=== All Tests Passed ==="
