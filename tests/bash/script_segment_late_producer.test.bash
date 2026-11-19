#!/usr/bin/env bash
# tests/bash/script_segment_late_producer.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A script segment whose producer reports its process group after the
# deadline (a producer scheduled late on a loaded host) must still get TERM,
# the grace and KILL, and must not outlive the prompt (jpease/gpy-archive#1252).
# The Bash 5 reader of gpy-agent/src/prompt/g_runner.bash learns each group
# from the producer p record; without one by the deadline it had nothing to
# signal, so a script that loops kept running, and its producer waited on it
# forever.
#
# The batch child overrides printf (a function wins over the builtin) to
# hold back just the p record by 400 ms against a 100 ms deadline: the
# late producer, every time, with no hook in the runner.

# shellcheck disable=SC2016  # the child shell expands its own program

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

[[ -n "${EPOCHREALTIME:-}" ]] || test_skip "needs Bash 5: the late-record path is the clock reader"
RUNNER="$ROOT/gpy-agent/src/prompt/g_runner.bash"

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-late.XXXXXX")"
T="$(cd "$T" && pwd -P)"
# Marks the sleep of the looping script, so the survivor check sees only ours.
MARK="9.$$"
cleanup() {
    [[ -s "$T/loop.pid" ]] && kill -KILL "$(<"$T/loop.pid")" 2>/dev/null
    pkill -KILL -f "sleep $MARK" 2>/dev/null
    rm -rf "$T"
}
trap cleanup EXIT

# Notes its PID, then a TERM in a file, and keeps going: only KILL ends it.
printf '#!/bin/sh\necho $$ >"%s/loop.pid"\ntrap "echo term >\\"%s/loop.term\\"" TERM\nwhile :; do sleep %s; done\n' \
    "$T" "$T" "$MARK" >"$T/loop.sh"
chmod 755 "$T/loop.sh"

# One 100 ms segment, run by the runner in this Bash with a delayed p record.
"$BASH" --noprofile --norc -c '
    printf() {
        [[ $1 == "\\036%s p "* ]] && /bin/sleep 0.4
        builtin printf "$@"
    }
    source "$1" && shift && __gpy_g_batch "$@"
' gpy "$RUNNER" /bin/sh late 100 "" "$T/loop.sh" "" 0 >"$T/out" 2>/dev/null
out="$(<"$T/out")"

if [[ "$out" == *late$'\037'timeout$'\037'* ]]; then
    pass "the segment reports a timeout"
else
    fail "the segment should report a timeout, got: ${out//[$'\001'-$'\037']/|}"
fi

# The producer acts once its delayed record is written: TERM, 50 ms, KILL.
term_seen() { [[ -e "$T/loop.term" ]]; }
if shell_e2e_poll 5 term_seen; then
    pass "the script got TERM from its producer (the grace before KILL)"
else
    fail "the script never got TERM: the late producer did not signal its group"
fi

# shellcheck disable=SC2009  # pgrep -c is not on macOS; [s] keeps grep off itself
survivors() { ps -axo command= | grep -c "[s]leep $MARK"; }
gone() { [[ "$(survivors)" == 0 ]] && ! kill -0 "$(<"$T/loop.pid")" 2>/dev/null; }
if [[ ! -s "$T/loop.pid" ]]; then
    fail "the script never started (no PID noted)"
elif shell_e2e_poll 5 gone; then
    pass "the script and its sleep are gone: nothing outlives the prompt"
else
    fail "the script survived its deadline ($(survivors) sleep(s) left)"
fi

test_finish "a late producer still enforces its segment deadline"
