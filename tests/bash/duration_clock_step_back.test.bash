#!/usr/bin/env bash
# tests/bash/duration_clock_step_back.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A clock that steps back while a command runs makes its elapsed time
# negative. `--duration-ms` is a u64, so passing it made `gpy prompt` exit 2
# and the prompt fall back to the bare symbol: the hook passes no duration
# then, as for a prompt with no command. A stub `gpy` on PATH logs its argv
# and, as clap does, exits 2 on a `--duration-ms` that is not a u64.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-clock-step.XXXXXX")" || exit 1
trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/gpy" <<STUB
#!/bin/sh
echo "\$*" >'$tmp/argv'
while [ \$# -gt 0 ]; do
    case "\$1:\$2" in --duration-ms:*[!0-9]* | --duration-ms:) exit 2 ;; esac
    shift
done
printf 'STUB> '
STUB
chmod +x "$tmp/gpy"
PATH="$tmp:$PATH"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash
# Whole seconds from a faked clock: the elapsed time is deterministic.
__gpy_clock=seconds
__gpy_now_into() { printf -v "$1" '%s' "$fake_now"; }

# prompt_after START: a prompt at clock 100 after a command started at START.
prompt_after() {
    fake_now=100
    __gpy_cmd_start="$1"
    __gpy_precmd
}

prompt_after 98
if [[ $(<"$tmp/argv") == *" --duration-ms 2000"* ]]; then
    pass "a command that ran passes its duration"
else
    fail "a command that ran passes no duration: $(<"$tmp/argv")"
fi

prompt_after 105
argv="$(<"$tmp/argv")"
if [[ $argv != *--duration-ms* ]]; then
    pass "a clock that stepped back passes no duration"
else
    fail "a clock that stepped back passed a duration: $argv"
fi
check "the prompt is the binary's" "STUB> " "$PS1"

test_finish "a backwards clock step keeps the prompt"
