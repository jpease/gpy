#!/usr/bin/env zsh
# tests/zsh/hostile_env.test.zsh
#
# Hostile-shell-environment harness (zsh counterpart of
# tests/bash/hostile_env.test.bash). Every scenario starts a FRESH child zsh
# whose rc file first builds a hostile environment, then sources gpy.zsh, then
# runs a few commands and asserts the user's environment survived. New
# hostile-environment issues add a `scenario_<name>` function plus one
# `run_scenario` line below.
#
# Children never start a gpy-agent: the supervisor is disabled and the socket
# path points at a file that does not exist. Paths reach the child only as
# environment/argv, never interpolated into code.

ROOT=${0:a:h:h:h}
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-hostile.XXXXXX") || exit 1
cleanup() { rm -rf "$SANDBOX"; }
trap cleanup EXIT
mkdir -p "$SANDBOX/cfg" "$SANDBOX/cache" "$SANDBOX/run"

failures=0
fail() { print -r -- "FAIL: $*"; failures=$((failures + 1)); }
pass() { print -r -- "PASS: $*"; }

# run_child <prelude> <input>: sets CHILD_OUT to the child's merged output.
# The rc file runs <prelude>, then sources gpy.zsh from $GPY_TEST_ROOT; <input>
# is piped to the interactive child, one command per line.
run_child() {
    local zdot="$SANDBOX/zdot"
    mkdir -p "$zdot"
    {
        print -r -- "$1"
        print -r -- 'source "$GPY_TEST_ROOT/zsh/gpy.zsh"'
    } >"$zdot/.zshrc"
    CHILD_OUT="$(print -r -- "$2" | env -i \
        PATH="${GPY_TEST_STUBS:+$GPY_TEST_STUBS:}/usr/bin:/bin" TERM=dumb HOME="$SANDBOX" \
        ZDOTDIR="$zdot" \
        XDG_CONFIG_HOME="$SANDBOX/cfg" XDG_CACHE_HOME="$SANDBOX/cache" \
        XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" \
        zsh -i 2>&1)"
}

# field <name>: value printed by the child as `name=[value]`.
field() { print -r -- "$CHILD_OUT" | sed -n "s/.*[ m]$1=\[\([^]]*\)\].*/\1/p" | sed -n 1p; }

run_scenario() {
    print -r -- "=== Scenario: $1 ==="
    "scenario_$1"
}

nounset_scenario() {
    local label=$1 unsets=$2
    local repo="$SANDBOX/nu-repo" stubs="$SANDBOX/nu-stubs"
    mkdir -p "$repo" "$stubs"
    git -C "$repo" init -q
    printf '#!/bin/sh\nexit 1\n' >"$stubs/gpy-agent"
    chmod +x "$stubs/gpy-agent"
    GPY_TEST_STUBS=$stubs run_child "
setopt nounset
unset GPY_AGENT_SOCKET_PATH $unsets" 'print -r -- "ep=[$(__gpy_ipc_endpoint)]"
print -r -- "ic=[$(__gpy_instant_cache_dir)]"
cd "$HOME/nu-repo"
true
false
cd /
cd "$HOME/nu-repo"
print -r -- "hook=[${zshexit_functions[(I)__gpy_zshexit]}]"
exit'
    if [[ "$CHILD_OUT" == *"parameter not set"* ]]; then
        fail "$label: nounset produced errors:"
        print -r -- "$CHILD_OUT" | grep 'parameter not set' | sort | uniq -c
        return
    fi
    local hook ok=1 f
    hook=$(field hook)
    [[ -n "$hook" && "$hook" -ge 1 ]] || { ok=0; fail "$label: __gpy_zshexit not registered (hook=[$hook])"; }
    for f in ep ic; do
        [[ "$(field $f)" == /* ]] || { ok=0; fail "$label: $f=[$(field $f)] is not absolute"; }
    done
    ((ok)) && pass "$label: no parameter-not-set errors, exit hook registered"
}

scenario_nounset_no_xdg() {
    nounset_scenario "setopt nounset, no XDG vars" "XDG_RUNTIME_DIR XDG_CACHE_HOME"
}

scenario_nounset_runtime_only() {
    nounset_scenario "setopt nounset, only XDG_RUNTIME_DIR" "XDG_CACHE_HOME"
}

scenario_cache_dir_lifecycle() {
    local -a d
    rm -rf "$SANDBOX"/gpy_cache_*(N)
    run_child '' $'source "$GPY_TEST_ROOT/zsh/gpy.zsh"\nexit'
    d=("$SANDBOX"/gpy_cache_*(N))
    ((${#d} == 0)) && pass "re-source + exit leaves no gpy_cache_*" \
        || fail "re-source + exit left ${#d} gpy_cache_* dir(s)"
    run_child '' $'exec zsh -i -c true'
    d=("$SANDBOX"/gpy_cache_*(N))
    ((${#d} == 0)) && pass "exec leaves no gpy_cache_*" \
        || fail "exec left ${#d} gpy_cache_* dir(s)"
}

run_scenario nounset_no_xdg
run_scenario nounset_runtime_only
run_scenario cache_dir_lifecycle

if ((failures > 0)); then
    print -r -- "=== $failures scenario assertion(s) failed ==="
    exit 1
fi
print -r -- "=== All Tests Passed ==="
