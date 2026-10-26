#!/usr/bin/env bash
# tests/bash/supervisor_check_no_date_fork.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_supervisor_check runs after every prompt. Its clock read precedes the
# rate limiter, so on bash 4.2+ it must use the builtin epoch: a PATH-wrapped
# `date` that logs each launch sees none for throttled checks. Bash 3.2 keeps
# the `date` fallback, and the limiter admits a check on the same schedule
# either way.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-sup-date.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

export GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1
export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=30

DATE_LOG="$SANDBOX/date.log"
PROBE_LOG="$SANDBOX/probe.log"
real_date="$(command -v date)"
mkdir -p "$SANDBOX/bin"
cat >"$SANDBOX/bin/date" <<EOF
#!/bin/sh
echo date >>"$DATE_LOG"
exec "$real_date" "\$@"
EOF
chmod +x "$SANDBOX/bin/date"

# A healthy agent: the check that passes the limiter probes it exactly once.
# shellcheck disable=SC2329 # invoked by __gpy_supervisor_check
__gpy_supervisor_is_running() {
    echo probe >>"$PROBE_LOG"
}

count() { [[ -f "$1" ]] && wc -l <"$1" | tr -d ' ' || echo 0; }

# run_checks EPOCH_FLAG: ten checks just after one ran, then one whose window
# has elapsed. Prints "<date launches during throttled checks> <probes>".
run_checks() {
    local __gpy_have_printf_epoch="$1" i throttled_dates
    rm -f "$DATE_LOG" "$PROBE_LOG"
    __gpy_supervisor_last_check_time="$("$real_date" +%s)"
    for ((i = 0; i < 10; i++)); do
        PATH="$SANDBOX/bin:$PATH" __gpy_supervisor_check
    done
    throttled_dates="$(count "$DATE_LOG")"
    __gpy_supervisor_last_check_time=$(($("$real_date" +%s) - GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS))
    PATH="$SANDBOX/bin:$PATH" __gpy_supervisor_check
    printf '%s %s' "$throttled_dates" "$(count "$PROBE_LOG")"
}

if [[ "$__gpy_have_printf_epoch" == "1" ]]; then
    got="$(run_checks 1)"
    if [[ "$got" == "0 1" ]]; then
        pass "bash $BASH_VERSION: throttled checks launch no date; an elapsed window probes once"
    else
        fail "bash $BASH_VERSION builtin epoch: want '0 1' (date launches, probes), got '$got'"
    fi
fi

# The pre-4.2 path, forced on any bash: one date per check, same schedule.
got="$(run_checks 0)"
if [[ "$got" == "10 1" ]]; then
    pass "date fallback: one date per throttled check; an elapsed window probes once"
else
    fail "date fallback: want '10 1' (date launches, probes), got '$got'"
fi

test_finish "supervisor check reads the clock without forking date on bash 4.2+"
