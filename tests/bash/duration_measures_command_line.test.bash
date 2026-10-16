#!/usr/bin/env bash
# tests/bash/duration_measures_command_line.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The command start time is armed once per command line (__gpy_arm_preexec), not per DEBUG-trap firing.

# shellcheck disable=SC2016
# Single quotes are load-bearing: the child shell expands them.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if (( BASH_VERSINFO[0] < 5 )); then
    test_skip "bash $BASH_VERSION has no EPOCHREALTIME duration method"
fi

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-duration.XXXXXX")"
cleanup() { rm -rf "$T"; }
trap cleanup EXIT
# Child's config disables the supervisor; the theme export re-sets the env flag.
source "$ROOT/tests/lib/supervisor_off.bash" "$T"

# Scrub every GPY_* variable inherited from the developer's shell.
gpy_scrub=()
while IFS='=' read -r __gpy_env_name _; do
    case "$__gpy_env_name" in
        GPY_*) gpy_scrub+=(-u "$__gpy_env_name") ;;
    esac
done < <(env)

# make_rc NAME BEFORE AFTER: rc that runs BEFORE, sources gpy.bash, writes $T/NAME.pid, runs AFTER.
make_rc() {
    printf '%s\nsource "%s/bash/gpy.bash"\necho "$$" >"%s/%s.pid"\n%s\n' \
        "$2" "$ROOT" "$T" "$1" "$3" >"$T/$1.rc"
}

# run_session NAME FEEDER: interactive bash on rc NAME fed FEEDER's stdout.
run_session() {
    "$2" | env "${gpy_scrub[@]}" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$T/missing.sock" \
        HOME="$T" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config" \
        XDG_RUNTIME_DIR="$T/run" TMPDIR="$T" \
        PATH=/usr/bin:/bin TERM=dumb GPY_TEST_ENTRY="$ROOT/bash/gpy.bash" \
        "$BASH" --noprofile --rcfile "$T/$1.rc" -i 2>&1
}

# `report OUTPUT KEY` -- the value the session printed as KEY=<n>.
report() {
    printf '%s\n' "$1" | grep -oE "$2=[0-9-]+" | tail -n 1 | cut -d= -f2
}

# `expect_at_least|expect_below OUTPUT KEY LIMIT LABEL`
expect_at_least() {
    local value
    value="$(report "$1" "$2")"
    if [[ -n "$value" && "$value" -ge "$3" ]]; then
        pass "$4: ${value} ms"
    else
        fail "$4: ${value:-no value} ms, expected >= $3"
    fi
}
expect_below() {
    local value
    value="$(report "$1" "$2")"
    if [[ -n "$value" && "$value" -lt "$3" ]]; then
        pass "$4: ${value} ms"
    else
        fail "$4: ${value:-no value} ms, expected < $3"
    fi
}

echo "=== another PROMPT_COMMAND entry does not reset the start time ==="
make_rc preset "PROMPT_COMMAND='true'" ""
feed_preset() {
    printf '%s\n' 'sleep 1.2' 'echo "D_sleep=$__gpy_cmd_duration"' 'exit'
}
out="$(run_session preset feed_preset)"
expect_at_least "$out" D_sleep 1100 "sleep 1.2 with PROMPT_COMMAND='true' preset"

echo "=== a list and a loop count from their first command ==="
make_rc plain "" ""
feed_lists() {
    printf '%s\n' 'sleep 1.2; true' 'echo "D_list=$__gpy_cmd_duration"' \
        'for i in 1 2 3; do sleep 0.4; done' 'echo "D_loop=$__gpy_cmd_duration"' 'exit'
}
out="$(run_session plain feed_lists)"
expect_at_least "$out" D_list 1100 "sleep 1.2; true"
expect_at_least "$out" D_loop 1100 "for i in 1 2 3; do sleep 0.4; done"

echo "=== a SIGURG during the command does not reset the start time ==="
feed_urg() {
    # This session's rc writes the PID file once gpy is loaded.
    shell_e2e_poll 10 test -s "$T/plain.pid" || return
    printf '%s\n' 'sleep 1.5'
    # Doorbell lands 0.5 s into the 1.5 s command; report typed after it ends.
    sleep 0.5
    kill -URG "$(cat "$T/plain.pid")"
    sleep 1.5
    printf '%s\n' 'echo "D_urg=$__gpy_cmd_duration"' 'exit'
}
rm -f "$T/plain.pid"
out="$(run_session plain feed_urg)"
expect_at_least "$out" D_urg 1400 "SIGURG 0.5 s into sleep 1.5"

echo "=== idle time at the prompt is never counted ==="
feed_idle() {
    printf '%s\n' 'true'
    # Idle at the prompt for 1.5 s; none of it may be counted.
    sleep 1.5
    printf '%s\n' 'true' 'echo "D_idle=$__gpy_cmd_duration"' 'exit'
}
out="$(run_session plain feed_idle)"
expect_below "$out" D_idle 200 "true after 1.5 s idle"

make_rc appended "" "PROMPT_COMMAND+='; :'"
out="$(run_session appended feed_idle)"
expect_below "$out" D_idle 200 "true after 1.5 s idle, '; :' appended to PROMPT_COMMAND after gpy"

echo "=== re-sourcing adds no second arming hook ==="
feed_resource() {
    printf '%s\n' 'source "$GPY_TEST_ENTRY"' 'source "$GPY_TEST_ENTRY"' \
        '__c="${PROMPT_COMMAND[*]}"; __n=0; while [[ "$__c" == *__gpy_arm_preexec* ]]; do __c="${__c#*__gpy_arm_preexec}"; __n=$((__n + 1)); done; echo "HOOKS=$__n"' \
        'exit'
}
out="$(run_session plain feed_resource)"
if [[ "$(report "$out" HOOKS)" == "1" ]]; then
    pass "string PROMPT_COMMAND holds one arming hook after re-sourcing"
else
    fail "string PROMPT_COMMAND holds $(report "$out" HOOKS) arming hooks after re-sourcing"
fi

if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then
    echo "=== array PROMPT_COMMAND (bash >= 5.1) ==="
    make_rc array "PROMPT_COMMAND=(true)" ""
    out="$(run_session array feed_preset)"
    expect_at_least "$out" D_sleep 1100 "sleep 1.2 with PROMPT_COMMAND=(true) preset"
    out="$(run_session array feed_resource)"
    if [[ "$(report "$out" HOOKS)" == "1" ]]; then
        pass "array PROMPT_COMMAND holds one arming hook after re-sourcing"
    else
        fail "array PROMPT_COMMAND holds $(report "$out" HOOKS) arming hooks after re-sourcing"
    fi

    make_rc array_appended "PROMPT_COMMAND=(true)" "PROMPT_COMMAND+=(':')"
    out="$(run_session array_appended feed_idle)"
    expect_below "$out" D_idle 200 "true after 1.5 s idle, ':' element appended after gpy"
fi

test_finish
