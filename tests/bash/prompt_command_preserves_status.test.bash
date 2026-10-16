#!/usr/bin/env bash
# tests/bash/prompt_command_preserves_status.test.bash
# __gpy_precmd must preserve `$?` for later PROMPT_COMMAND hooks. Each case runs a
# fresh interactive child bash (3.2 and current) that never starts an agent.

# shellcheck disable=SC2016
# Single quotes are load-bearing: rc preludes and hooks are literal child code.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-pcstatus.XXXXXX")"
cleanup() { rm -rf "$SANDBOX"; }
trap cleanup EXIT
# The child's config dir disables the supervisor (the theme export would re-enable it).
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
mkdir -p "$SANDBOX/run"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# run_child <bash> <prelude> <trailer> <input>: sets CHILD_OUT; one prompt cycle
# per input line; `env -i` scrubs GPY_*.
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

# seen <label> [n]: last n (default 2) values; the startup prompt cycle precedes any command.
seen() {
    printf '%s\n' "$CHILD_OUT" | sed -n "s/^$1=//p" | tail -n "${2:-2}" | tr '\n' ' ' | sed 's/ $//'
}

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

    run_child "$bin" 'PROMPT_COMMAND='"'"'echo "DS=$?"'"'" '' 'false
(exit 42)
exit'
    expect "$label, entry before gpy" DS "1 42"

    run_child "$bin" '' 'PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }"'"'"'echo "AFT=$?"'"'" 'false
(exit 42)
exit'
    expect "$label, entry after gpy" AFT "1 42"

    # (BEF preserves `$?` itself: a bare `echo` would reset it for AFT.)
    run_child "$bin" '__bef() { local s=$?; echo "BEF=$s"; return "$s"; }
PROMPT_COMMAND=__bef' \
        'PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }"'"'"'echo "AFT=$?"'"'" 'false
(exit 42)
exit'
    expect "$label, both sides" BEF "1 42"
    expect "$label, both sides" AFT "1 42"

    # The first prompt cycle also carries the status of the last rc command.
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
        # bash restores `$?` between array elements; A1 carries gpy's precmd.
        expect "$label, array element 0" A1 "1 42"
        expect "$label, array element 1" A2 "1 42"
    else
        echo "N/A: array PROMPT_COMMAND needs bash >= 5.1 ($label)"
    fi
done

test_finish
