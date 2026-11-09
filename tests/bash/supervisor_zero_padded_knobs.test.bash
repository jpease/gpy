#!/usr/bin/env bash
# tests/bash/supervisor_zero_padded_knobs.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zero-padded supervisor knobs are decimal, as in Zsh and Fish: 08 is 8 (not an
# octal error that disables the limiter and the restart cap), 010 is 10.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-sup-pad.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

export GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1

PROBE_LOG="$SANDBOX/probe.log"
ERR="$SANDBOX/err.log"
# shellcheck disable=SC2329 # invoked by __gpy_supervisor_check
__gpy_supervisor_is_running() {
    echo probe >>"$PROBE_LOG"
}

probes() { [[ -f "$PROBE_LOG" ]] && wc -l <"$PROBE_LOG" | tr -d ' ' || echo 0; }

# check_after SECONDS INTERVAL: one check SECONDS after the last one.
check_after() {
    rm -f "$PROBE_LOG"
    __gpy_supervisor_last_check_time=$(($(date +%s) - $1))
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS="$2" __gpy_supervisor_check 2>"$ERR"
}

check_after 3 08
check "interval 08 throttles a check 3 s after the last one" "0 " "$(probes) $(cat "$ERR")"
check_after 9 010
check "interval 010 is ten seconds, not octal eight: 9 s still throttles" "0 " "$(probes) $(cat "$ERR")"
check_after 11 010
check "interval 010 admits a check 11 s after the last one" "1 " "$(probes) $(cat "$ERR")"

# shellcheck disable=SC2329 # invoked by __gpy_supervisor_check
__gpy_supervisor_is_running() { return 1; }
# shellcheck disable=SC2329 # invoked by __gpy_supervisor_check
__gpy_supervisor_start() { sleep 1; }
export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0 GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=08
# The check `disown`s its background start: it needs job control and a job
# that is still running (the stub sleeps) to stay quiet.
set -m
__gpy_supervisor_check_attempts=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
    __gpy_supervisor_last_check_time=0
    __gpy_supervisor_check 2>>"$ERR"
done
set +m
check "max attempts 08 caps at eight restarts" "8 " "$__gpy_supervisor_check_attempts $(cat "$ERR")"

test_finish "zero-padded supervisor knobs are decimal"
