#!/usr/bin/env bash
# tests/bash/prompt_command_preserves_status.test.bash
#
# Regression test for #761: __gpy_precmd is prepended to PROMPT_COMMAND and
# used to return 0, so every user hook that ran after it saw `$?` = 0 instead
# of the command's exit status. Each case starts a FRESH interactive child bash
# (system bash 3.2 and the bash running this test), whose rc builds a
# PROMPT_COMMAND, sources gpy.bash, then is fed `false` / `(exit 42)`; the
# hooks print what `$?` they saw.
#
# Children never start a gpy-agent: the supervisor is disabled and the socket
# path does not exist. Paths reach the child only as environment/argv.

# shellcheck disable=SC2016
# Single quotes are load-bearing: rc preludes and hooks are literal child code.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-pcstatus.XXXXXX")"
cleanup() { rm -rf "$SANDBOX"; }
trap cleanup EXIT
# The child's XDG_CONFIG_HOME ($SANDBOX/config) disables the supervisor: the
# theme export would re-set the env flag below from config.toml (#836).
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
mkdir -p "$SANDBOX/run"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# run_child <bash> <prelude> <trailer> <input>: sets CHILD_OUT. The rc runs
# <prelude>, sources gpy.bash at top level, then runs <trailer>; <input> is
# piped to the interactive child, one prompt cycle per line. `env -i` scrubs
# every GPY_* variable the developer's shell may export.
run_child() {
    local bin="$1" rc="$SANDBOX/rc.bash"
    {
        printf '%s\n' "$2"
        printf '%s\n' 'source "$GPY_TEST_ROOT/bash/gpy.bash"'
        printf '%s\n' "$3"
    } >"$rc"
    CHILD_OUT="$(printf '%s\n' "$4" | env -i \
        PATH=/usr/bin:/bin TERM=dumb HOME="$SANDBOX" \
        XDG_CONFIG_HOME="$SANDBOX/config" XDG_CACHE_HOME="$SANDBOX/cache" \
        XDG_RUNTIME_DIR="$SANDBOX/run" TMPDIR="$SANDBOX" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" \
        "$bin" --noprofile --rcfile "$rc" -i 2>&1)"
}

# seen <label> [n]: the last n (default 2) values printed as `label=value`,
# space-joined. The first prompt cycle runs at startup, before any command, so
# only the trailing values belong to the typed commands.
seen() {
    printf '%s\n' "$CHILD_OUT" | sed -n "s/^$1=//p" | tail -n "${2:-2}" | tr '\n' ' ' | sed 's/ $//'
}

# expect <what> <label> <want> [n]
expect() {
    local what="$1" label="$2" want="$3" got
    got="$(seen "$label" "${4:-2}")"
    if [[ "$got" == "$want" ]]; then
        pass "$what: $label saw [$got]"
    else
        fail "$what: $label saw [$got], want [$want]"
        printf '%s\n' "$CHILD_OUT"
    fi
}

array_supported() {
    "$1" -c '(( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) ))'
}

version_of() { "$1" -c 'printf "%s" "$BASH_VERSION"'; }

for bin in /bin/bash "$BASH"; do
    [[ -x "$bin" ]] || continue
    label="$bin ($(version_of "$bin"))"
    echo "=== $label ==="

    # An entry that was in PROMPT_COMMAND before gpy loaded (gpy prepends).
    run_child "$bin" 'PROMPT_COMMAND='"'"'echo "DS=$?"'"'" '' 'false
(exit 42)
exit'
    expect "$label, entry before gpy" DS "1 42"

    # An entry appended after gpy loaded.
    run_child "$bin" '' 'PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }"'"'"'echo "AFT=$?"'"'" 'false
(exit 42)
exit'
    expect "$label, entry after gpy" AFT "1 42"

    # Entries on both sides of gpy, string form.
    # (BEF preserves `$?` itself: a bare `echo` would reset it for AFT.)
    run_child "$bin" '__bef() { local s=$?; echo "BEF=$s"; return "$s"; }
PROMPT_COMMAND=__bef' \
        'PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }"'"'"'echo "AFT=$?"'"'" 'false
(exit 42)
exit'
    expect "$label, both sides" BEF "1 42"
    expect "$label, both sides" AFT "1 42"

    # The first prompt cycle also carries the status of the last rc command.
    # On bash >= 4 it runs gpy's one-shot DEBUG-trap capture entry first.
    run_child "$bin" 'PROMPT_COMMAND='"'"'echo "DS=$?"'"'" '(exit 7)' 'exit'
    expect "$label, first prompt" DS "7" 1

    # __gpy_arm_preexec runs last in PROMPT_COMMAND and must pass `$?` through.
    run_child "$bin" '' '' '(exit 5); __gpy_arm_preexec; echo "AP=$?"
exit'
    expect "$label, __gpy_arm_preexec" AP "5" 1

    if array_supported "$bin"; then
        run_child "$bin" 'PROMPT_COMMAND=('"'"'echo "A1=$?"'"'"' '"'"'echo "A2=$?"'"'"')' '' 'false
(exit 42)
exit'
        # bash restores `$?` between array elements, so A2 was always right;
        # A1 is the element gpy's precmd is prepended to.
        expect "$label, array element 0" A1 "1 42"
        expect "$label, array element 1" A2 "1 42"
    else
        echo "SKIP: array PROMPT_COMMAND needs bash >= 5.1 ($label)"
    fi
done

if ((failures > 0)); then
    echo "=== $failures assertion(s) failed ==="
    exit 1
fi
echo "=== All Tests Passed ==="
