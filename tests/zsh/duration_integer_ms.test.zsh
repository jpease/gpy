#!/usr/bin/env zsh
# tests/zsh/duration_integer_ms.test.zsh
#
# Regression test for #681: zsh computed the command duration in floating
# point from $EPOCHREALTIME and sent it as `"duration_ms":1205.82...`. The
# agent's `duration_ms` is a u64, so it rejected every duration request and
# the oneshot fallback's `--duration-ms` parser rejected the float too.
#
# Asserts that __gpy_cmd_duration is an integer number of milliseconds after
# a real preexec/precmd cycle, and that the duration request carries
# `"duration_ms":<digits>` with no decimal point.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-duration-integer.sock"

source zsh/gpy.zsh

tmp=$(mktemp -d)
failed=0

# One command cycle, run inside a single function so nothing re-records the
# start time between the steps. The start is backdated by 200 ms instead of
# sleeping, so the elapsed time is deterministic.
simulate_command() {
    __gpy_preexec "sleep 0.2"
    (( __gpy_cmd_start_time -= 200 ))
    __gpy_precmd
}
simulate_command
if [[ $__gpy_cmd_duration == <-> ]] && (( __gpy_cmd_duration >= 150 )); then
    print -r -- "PASS: __gpy_cmd_duration is integer milliseconds ($__gpy_cmd_duration)"
else
    print -r -- "FAIL: __gpy_cmd_duration is not integer milliseconds: '$__gpy_cmd_duration'"
    failed=1
fi

__gpy_send_json() { print -r -- "$1" > "$tmp/req"; }
__gpy_request_duration "$__gpy_cmd_duration" "" "" "" >/dev/null 2>&1
req=$(<"$tmp/req")
if [[ "$req" == *'"duration_ms":'<->','* ]]; then
    print -r -- "PASS: the duration request carries integer duration_ms"
else
    print -r -- "FAIL: the duration request does not carry integer duration_ms: $req"
    failed=1
fi

rm -rf "$tmp"
exit "$failed"
