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
# The child's XDG_CONFIG_HOME ($SANDBOX/config) disables the supervisor: the
# theme export would re-set the env flag below from config.toml.
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
mkdir -p "$SANDBOX/run"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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
        XDG_CONFIG_HOME="$SANDBOX/config" XDG_CACHE_HOME="$SANDBOX/cache" \
        XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" GPY_TEST_BIN="$ROOT/gpy-agent/target/debug" \
        "$BASH" --noprofile --rcfile "$rc" -i 2>&1)"
}

# field <name>: value printed by the child as `name=[value]`.
field() { printf '%s\n' "$CHILD_OUT" | sed -n "s/^$1=\[\(.*\)\]\$/\1/p" | sed -n 1p; }

run_scenario() {
    echo "=== Scenario: $1 ==="
    "scenario_$1"
}

scenario_prior_debug_trap_fires() {
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
    [[ -n "$(field after_dbg)" ]] || ok=0
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

# nounset_scenario <label> <unset-vars>: shared body for the `set -u` scenarios.
# The real gpy is on PATH, so every prompt runs it (the agent stays off).
nounset_scenario() {
    local label="$1" unsets="$2" repo="$SANDBOX/nu-repo"
    mkdir -p "$repo"
    git -C "$repo" init -q
    run_child "
set -u
PATH=\$GPY_TEST_BIN:\$PATH
unset GPY_AGENT_SOCKET_PATH $unsets" 'cd "$HOME/nu-repo"
true
false
cd /
cd "$HOME/nu-repo"
echo "bare=[$([[ $PS1 == "❯ " ]] && echo 1 || echo 0)]"
exit'
    if [[ "$CHILD_OUT" == *"unbound variable"* ]]; then
        fail "$label: set -u produced unbound-variable errors:"
        printf '%s\n' "$CHILD_OUT" | grep 'unbound variable' | sort | uniq -c
        return
    fi
    if [[ "$(field bare)" == 0 ]]; then
        pass "$label: no unbound-variable errors and gpy drew the prompt"
    else
        fail "$label: the prompt fell back to the bare symbol (bare=[$(field bare)])"
        printf '%s\n' "$CHILD_OUT"
    fi
}

scenario_nounset_no_xdg() {
    nounset_scenario "set -u, no XDG vars" "XDG_RUNTIME_DIR XDG_CACHE_HOME"
}

scenario_nounset_runtime_only() {
    nounset_scenario "set -u, only XDG_RUNTIME_DIR" "XDG_CACHE_HOME"
}

scenario_prompt_command_sees_status() {
    run_child '__pre() { local s=$?; echo "pre=[$s]"; return "$s"; }
PROMPT_COMMAND=__pre' 'PROMPT_COMMAND="${PROMPT_COMMAND}; "'"'"'echo "post=[$?]"'"'"'
false
(exit 42)
exit'
    local got
    got="$(printf '%s,%s' "$(printf '%s\n' "$CHILD_OUT" | sed -n 's/^pre=\[\(.*\)\]$/\1/p' | tail -n 2 | tr '\n' ' ')" \
        "$(printf '%s\n' "$CHILD_OUT" | sed -n 's/^post=\[\(.*\)\]$/\1/p' | tail -n 2 | tr '\n' ' ')")"
    if [[ "$got" == "1 42 ,1 42 " ]]; then
        pass "PROMPT_COMMAND entries before and after gpy see the command's status"
    else
        fail "PROMPT_COMMAND entries saw [$got], want [1 42 ,1 42 ] (pre,post)"
        printf '%s\n' "$CHILD_OUT"
    fi
}

# failglob makes an unmatched glob an error that aborts the EXIT handler; the
# shell exits without a refresh stamp, so gpy's sweep matches nothing.
scenario_failglob_exit_keeps_chain() {
    run_child 'shopt -s failglob
trap "echo chained-bye" EXIT' 'echo hi
exit'
    if [[ "$CHILD_OUT" == *"no match"* ]]; then
        fail "failglob: the EXIT handler hit an unmatched glob:"
        printf '%s\n' "$CHILD_OUT"
    elif [[ "$(printf '%s\n' "$CHILD_OUT" | grep -c '^chained-bye$')" -ne 1 ]]; then
        fail "failglob: the chained EXIT trap did not run exactly once:"
        printf '%s\n' "$CHILD_OUT"
    else
        pass "under failglob the EXIT handler still runs the chained trap"
    fi
}

# A printing `cd` wrapper and a CDPATH must not leak into the root gpy.bash
# resolves for itself (where it finds its completions).
scenario_cd_wrapper_keeps_root() {
    run_child 'cd() { builtin cd "$@" && echo CDNOISE; }
CDPATH=/tmp:/' 'echo "precmd=[$(declare -F __gpy_precmd)]"
echo "comp=[$(complete -p gpy 2>/dev/null)]"
exit'
    if [[ "$(field precmd)" != __gpy_precmd || "$(field comp)" != *_gpy* ]]; then
        fail "a cd wrapper or CDPATH broke the gpy.bash root:"
        printf '%s\n' "$CHILD_OUT"
    else
        pass "a printing cd wrapper and CDPATH leave the gpy.bash root intact"
    fi
}

run_scenario prior_debug_trap_fires
run_scenario exit_trap_with_quotes
run_scenario resource_keeps_chain
run_scenario never_chains_to_itself
run_scenario nounset_no_xdg
run_scenario nounset_runtime_only
run_scenario prompt_command_sees_status
run_scenario failglob_exit_keeps_chain
run_scenario cd_wrapper_keeps_root

test_finish
