#!/usr/bin/env zsh
# tests/zsh/hostile_env.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Hostile-shell-environment harness (zsh counterpart of
# tests/bash/hostile_env.test.bash). Every scenario starts a FRESH child zsh
# whose rc file first builds a hostile environment, then sources gpy.zsh, then
# runs a few commands and asserts the user's environment survived. New
# hostile-environment issues add a `scenario_<name>` function plus one
# `run_scenario` line below.
#
# Children run the real `gpy` (when built) and never start a gpy-agent:
# GPY_AGENT_ENABLED=0 is exported and the socket path points at a file that
# does not exist. Paths reach the child only as environment/argv, never
# interpolated into code.

ROOT=${0:a:h:h:h}
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-hostile.XXXXXX") || exit 1
cleanup() { rm -rf "$SANDBOX"; }
trap cleanup EXIT
mkdir -p "$SANDBOX/cfg/gpy" "$SANDBOX/cache" "$SANDBOX/run"
print -r -- $'[agent]\nenabled = false\n\n[agent.supervisor]\nenabled = false' >"$SANDBOX/cfg/gpy/config.toml"

emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

# The directory of the gpy under test, when there is one.
GPY_DIR=${commands[gpy]:h}

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
        PATH="${GPY_DIR:+$GPY_DIR:}/usr/bin:/bin" TERM=dumb HOME="$SANDBOX" \
        ZDOTDIR="$zdot" \
        XDG_CONFIG_HOME="$SANDBOX/cfg" XDG_CACHE_HOME="$SANDBOX/cache" \
        XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX" \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
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
    local repo="$SANDBOX/nu-repo"
    mkdir -p "$repo"
    git -C "$repo" init -q
    run_child "
setopt nounset
unset GPY_AGENT_SOCKET_PATH $unsets" 'cd "$HOME/nu-repo"
true
false
print -r -- "st=[$__gpy_status]"
cd /
cd "$HOME/nu-repo"
print -r -- "hook=[${zshexit_functions[(I)__gpy_zshexit]}]"
exit'
    if [[ "$CHILD_OUT" == *"parameter not set"* ]]; then
        fail "$label: nounset produced errors:"
        print -r -- "$CHILD_OUT" | grep 'parameter not set' | sort | uniq -c
        return
    fi
    local hook ok=1
    hook=$(field hook)
    [[ -n "$hook" && "$hook" -ge 1 ]] || { ok=0; fail "$label: __gpy_zshexit not registered (hook=[$hook])"; }
    [[ "$(field st)" == 1 ]] || { ok=0; fail "$label: the prompt did not see the failed status (st=[$(field st)])"; }
    ((ok)) && pass "$label: no parameter-not-set errors, status seen, exit hook registered"
}

scenario_nounset_no_xdg() {
    nounset_scenario "setopt nounset, no XDG vars" "XDG_RUNTIME_DIR XDG_CACHE_HOME"
}

scenario_nounset_runtime_only() {
    nounset_scenario "setopt nounset, only XDG_RUNTIME_DIR" "XDG_CACHE_HOME"
}

# urg_scenario <label> <prelude> [<extra input>]: a handler defined before
# GPY loads runs exactly once per SIGURG.
urg_scenario() {
    local label=$1 prelude=$2 extra=${3:-} n
    run_child "$prelude" "$extra"$'\n''kill -URG $$
exit'
    n=$(print -r -- "$CHILD_OUT" | grep -c 'user_urg=\[ran\]')
    if [[ "$CHILD_OUT" == *"parameter not set"* ]]; then
        fail "$label: nounset produced errors"
    elif [[ "$n" != 1 ]]; then
        fail "$label: user handler ran $n times: $CHILD_OUT"
    else
        pass "$label: user handler ran once"
    fi
}

scenario_user_trapurg_function_survives() {
    urg_scenario "user TRAPURG function" 'TRAPURG() { print -r -- "user_urg=[ran]" }'
}

scenario_user_urg_string_trap_survives() {
    urg_scenario "user URG string trap" $'setopt nounset\ntrap \'print -r -- "user_urg=[ran]"\' URG'
}

scenario_trapurg_resource_does_not_chain_to_itself() {
    urg_scenario "re-sourced gpy.zsh" 'TRAPURG() { print -r -- "user_urg=[ran]" }' \
        'source "$GPY_TEST_ROOT/zsh/gpy.zsh"'
}

run_scenario nounset_no_xdg
run_scenario nounset_runtime_only
run_scenario user_trapurg_function_survives
run_scenario user_urg_string_trap_survives
run_scenario trapurg_resource_does_not_chain_to_itself

test_finish
