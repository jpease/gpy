#!/usr/bin/env bash
# tests/bash/debug_trap.test.bash
#
# The DEBUG trap gpy installs for preexec emulation must
# (a) reference $COMP_LINE/$PROMPT_COMMAND only with a default, so it does not
#     error under `set -u` on every single command, and
# (b) chain any DEBUG trap already installed (e.g. bash-preexec, a user
#     framework) instead of overwriting it.

# shellcheck disable=SC2016
# Single quotes are load-bearing: assertions grep for literal `$VAR` text.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# Hermetic XDG config disables the agent and its supervisor.
dt_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-dt-xdg.XXXXXX")
# shellcheck source=tests/lib/supervisor_off.bash
source tests/lib/supervisor_off.bash "$dt_xdg_root"
export GPY_AGENT_SOCKET_PATH="$dt_xdg_root/missing.sock"

echo "=== Testing DEBUG trap chains a pre-existing trap ==="
prior_trap_fired=0
prior_trap() { prior_trap_fired=1; }
trap 'prior_trap' DEBUG

# shellcheck source=bash/gpy.bash
source bash/gpy.bash
# shellcheck disable=SC2154
# (__gpy_debug_oneshot and __gpy_prev_debug_trap are set by gpy.bash above.)

# Run gpy's one-shot PROMPT_COMMAND entry at this script's top level, the
# way bash would.
eval "$__gpy_debug_oneshot"
# The prior trap also fired on `source`; reset so only gpy's chain counts.
prior_trap_fired=0
if [[ "$__gpy_prev_debug_trap" != "prior_trap" ]]; then
    echo "FAIL: captured DEBUG body is [$__gpy_prev_debug_trap], want [prior_trap]"
    exit 1
fi
BASH_COMMAND=':'
__gpy_debug_trap
if [[ "$prior_trap_fired" -ne 1 ]]; then
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

echo "=== Testing EXIT trap containing quotes chains ==="
for gpy_bash_bin in /bin/bash "$BASH"; do
    out=$("$gpy_bash_bin" -c 'trap "echo '"'"'bye'"'"'" EXIT; source "$1/bash/gpy.bash"; exit' _ "$ROOT" 2>&1)
    if [[ "$out" != *bye* || "$out" == *"unexpected EOF"* ]]; then
        echo "FAIL: EXIT trap with quotes under $gpy_bash_bin: $out"
        exit 1
    fi
done
echo "PASS: EXIT trap containing quotes runs at exit"

echo "=== Testing EXIT trap sees the real exit status ==="
for gpy_bash_bin in /bin/bash "$BASH"; do
    for load_gpy in 0 1; do
        gpy_script='trap '"'"'printf "status=%s\n" "$?"'"'"' EXIT'$'\n'
        if [[ "$load_gpy" == 1 ]]; then
            gpy_script+='source "$1/bash/gpy.bash"'$'\n'
        fi
        gpy_script+='exit 7'
        out=$("$gpy_bash_bin" --noprofile --norc -c "$gpy_script" _ "$ROOT" 2>/dev/null)
        code=$?
        if [[ "$out" != *"status=7" || "$code" -ne 7 ]]; then
            echo "FAIL: $gpy_bash_bin load_gpy=$load_gpy: trap saw [$out], exit code $code (want status=7, 7)"
            exit 1
        fi
    done
done
echo "PASS: pre-existing EXIT trap observes the incoming exit status"

echo "=== Testing DEBUG trap is safe under set -u ==="
OUT_LOG="$(mktemp)"
cleanup() { rm -f "$OUT_LOG"; }
trap cleanup EXIT

for gpy_bash_bin in /bin/bash "$BASH"; do
    env -u COMP_LINE "$gpy_bash_bin" --noprofile --norc -c '
        set -u
        cd "$1" || exit 1
        source bash/gpy.bash
        eval "$__gpy_debug_oneshot"
        BASH_COMMAND=":"
        __gpy_debug_trap
        __gpy_arm
        BASH_COMMAND="true"
        __gpy_debug_trap
    ' _ "$ROOT" >"$OUT_LOG" 2>&1
    status=$?

    if [[ $status -ne 0 ]]; then
        echo "FAIL: DEBUG trap errored under set -u with $gpy_bash_bin (exit $status):"
        cat "$OUT_LOG"
        exit 1
    fi
    if grep -qi "unbound variable" "$OUT_LOG"; then
        echo "FAIL: DEBUG trap referenced an unset variable under set -u with $gpy_bash_bin:"
        cat "$OUT_LOG"
        exit 1
    fi
done
echo "PASS: DEBUG trap produces zero errors under set -u"

echo "=== Testing a bind -x key press keeps the preexec arm ==="
# Bash sets READLINE_LINE/READLINE_POINT only while a `bind -x` function
# runs; its DEBUG firing is a key press, not the user's command, so the arm
# (and the start time it takes) must wait for the real command. The live
# DEBUG trap is removed first, so only the explicit calls below fire it.
trap - DEBUG
__gpy_armed=1
__gpy_cmd_start=""
READLINE_LINE="" READLINE_POINT=0
BASH_COMMAND="__my_widget"
__gpy_debug_trap
if [[ "$__gpy_armed" != 1 || -n "$__gpy_cmd_start" ]]; then
    echo "FAIL: a bind -x DEBUG firing consumed the preexec arm"
    exit 1
fi
unset READLINE_LINE READLINE_POINT
BASH_COMMAND="sleep 1"
__gpy_debug_trap
if [[ -n "$__gpy_armed" ]]; then
    echo "FAIL: the user's command did not consume the preexec arm"
    exit 1
fi
echo "PASS: a bind -x key press keeps the preexec arm for the real command"

rm -rf "$dt_xdg_root"
echo "=== All Tests Passed ==="
