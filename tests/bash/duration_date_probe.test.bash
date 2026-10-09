#!/usr/bin/env bash
# tests/bash/duration_date_probe.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash 4 has no EPOCHREALTIME, so durations come from `date +%s%N` -- but only
# GNU date knows `%N`. BSD/macOS date (Homebrew's Bash 4 on a Mac) prints a
# literal `N`, and the old code then fed `1730000000N` into `$(( ))` on every
# prompt (#850). The method is now chosen by probing `date` once.
#
# A Bash 4 interpreter is not required: __gpy_select_duration_method takes the
# major version as an argument, and `date` is a PATH shim that behaves like
# BSD or GNU date on demand.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-duration-probe.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

FAILED=0
check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

real_date="$(command -v date)"
mkdir -p "$SANDBOX/bsd" "$SANDBOX/gnu"
# BSD date: %N is not a conversion, so it is printed as the letter N.
cat >"$SANDBOX/bsd/date" <<EOF
#!/bin/sh
if [ "\$1" = "+%s%N" ]; then
    echo "\$("$real_date" +%s)N"
else
    exec "$real_date" "\$@"
fi
EOF
# GNU date: nanoseconds, nine digits after the seconds.
cat >"$SANDBOX/gnu/date" <<EOF
#!/bin/sh
if [ "\$1" = "+%s%N" ]; then
    echo "\$("$real_date" +%s)123456789"
else
    exec "$real_date" "\$@"
fi
EOF
chmod +x "$SANDBOX/bsd/date" "$SANDBOX/gnu/date"

method_with() {
    local shim="$1" major="$2" out
    PATH="$shim:$PATH" __gpy_select_duration_method "$major" out
    printf '%s' "$out"
}

check "Bash 4 with BSD date falls back to whole seconds" date-seconds "$(method_with "$SANDBOX/bsd" 4)"
check "Bash 4 with GNU date keeps nanosecond timing" date "$(method_with "$SANDBOX/gnu" 4)"
check "Bash 5 uses EPOCHREALTIME whatever date does" epochrealtime "$(method_with "$SANDBOX/bsd" 5)"
check "Bash 3 stays disabled" none "$(method_with "$SANDBOX/gnu" 3)"

# The whole-seconds method must yield a sane duration, not an arithmetic error.
# __gpy_precmd's rendering and agent bookkeeping are out of scope here.
# shellcheck disable=SC2329 # invoked by __gpy_precmd
__gpy_render_prompt() { :; }
# shellcheck disable=SC2329
__gpy_register_with_agent() { :; }
# shellcheck disable=SC2329
__gpy_sync_workspace() { :; }
# shellcheck disable=SC2329
__gpy_supervisor_check() { :; }
# shellcheck disable=SC2329
__gpy_consume_shell_flags() { :; }
# shellcheck disable=SC2329
__gpy_keep_arm_hook_last() { :; }

__gpy_duration_method=date-seconds
__gpy_cmd_duration=""
PATH="$SANDBOX/bsd:$PATH" __gpy_preexec
sleep 2
PATH="$SANDBOX/bsd:$PATH" __gpy_precmd 2>"$SANDBOX/precmd.err" >/dev/null
check "date-seconds precmd prints no arithmetic error" "" "$(cat "$SANDBOX/precmd.err")"
if [[ "$__gpy_cmd_duration" =~ ^[0-9]+$ ]] && ((__gpy_cmd_duration >= 1000 && __gpy_cmd_duration <= 3000)); then
    echo "PASS: a ~2s command measures ${__gpy_cmd_duration}ms with whole-second timing"
else
    echo "FAIL: a ~2s command measured [$__gpy_cmd_duration]ms (want 1000-3000)"
    FAILED=1
fi

exit "$FAILED"
