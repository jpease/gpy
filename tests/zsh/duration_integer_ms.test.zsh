#!/usr/bin/env zsh
# tests/zsh/duration_integer_ms.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The command duration is an integer number of milliseconds: the binary's
# `--duration-ms` is a u64, so a floating-point value computed from
# $EPOCHREALTIME would be rejected. A prompt with no command before it (empty
# Enter) passes no duration at all.
#
# A real preexec/precmd cycle against a stub gpy that logs its argv.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-duration.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
print -r -- "#!/bin/sh
echo \"\$*\" >'$tmp/argv'
printf '> \\037'" >"$tmp/gpy"
chmod +x "$tmp/gpy"
path=("$tmp" $path)

source zsh/gpy.zsh

# One command cycle and then a prompt with no command, run inside a single
# function: a zsh script runs preexec hooks before each top-level command,
# which would re-record the start time between the steps. The start is
# backdated by 200 ms instead of sleeping, so the elapsed time is
# deterministic.
simulate_command() {
    __gpy_preexec "sleep 0.2"
    (( __gpy_cmd_start -= 200 ))
    __gpy_precmd
    duration=$__gpy_duration
    args=$(<"$tmp/argv")
    __gpy_precmd
    args_empty=$(<"$tmp/argv")
}
simulate_command
if [[ $duration == <-> ]] && (( duration >= 150 )); then
    pass "the duration is integer milliseconds ($duration)"
else
    fail "the duration is not integer milliseconds: '$duration'"
fi
if [[ $args == *" --duration-ms "<->(| *) ]]; then
    pass "the binary gets an integer --duration-ms"
else
    fail "the binary does not get an integer --duration-ms: $args"
fi

if [[ $args_empty != *--duration-ms* ]]; then
    pass "no command, no duration"
else
    fail "a prompt with no command passed a duration: $args_empty"
fi

test_finish "the duration is integer milliseconds"
