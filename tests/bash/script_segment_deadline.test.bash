#!/usr/bin/env bash
# tests/bash/script_segment_deadline.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The script (G) segment deadline through the real prompt hook: a due prompt
# (`gpy prompt bash`, which runs the embedded runner in a Bash 4+ child) whose
# batch holds a hung script, one that ignores TERM and keeps its stdout, and
# a fast one, all with timeout_ms T, never draws before T (the scripts really
# ran), the fast segment still shows, both hung ones are hidden, and no
# script process survives. A latency test, not a benchmark:
# scripts/ci-bench.sh measures cost.
#
# Mixed timeouts: one runner batch (gpy-agent/src/prompt/g_runner.bash,
# sourced straight into Bash here) holds a 500 ms hung script and a 100 ms
# script that catches TERM and keeps running, in both orders. The short one
# must get TERM by its own deadline (not the long one), then the grace (its
# TERM handler runs) and KILL soon after. Hang-type bounds, always enforced (a
# miss is retried, 3 attempts, because a stalled shared runner misses once in
# a while where a serial reader misses every time): TERM and KILL both before
# the long segment's deadline, which a reader waiting on the long segment
# misses. Strict bounds: TERM within T + 150 ms, KILL within the 50 ms grace +
# 100 ms of it, and 25 ms of the grace seen between the two.
#
# Two latency bounds. Strict: p99 (nearest rank over RUNS due prompts per
# shell) within T + 50 ms grace + 30 ms, the quiet-host acceptance bound.
# Hang: every prompt within T + 500 ms, which only a missed
# deadline breaks. GPY_STRICT_DEADLINE=1 enforces both; 0 enforces the hang
# bound and only reports strict. The default is 1, or 0 under CI: shared
# runners (macOS above all) are too noisy for strict. The advisory
# strict step of the Performance budgets job in pr-gate.yml runs this test with
# GPY_STRICT_DEADLINE=1.
#
# Shells: this Bash and /bin/bash when it is another version (macOS 3.2); the
# runner child is the Bash 4+ `gpy prompt` finds on PATH. The mixed batch
# also runs in /bin/bash when it is Bash < 4 (the timer-group path). Each
# child prints S right before a due prompt and E after it; this Bash stamps
# both lines as they arrive, so no clock is needed inside the child.
#
# GPY_TEST_G_LATENCY_RUNS (default 20) sets RUNS. Needs Bash 5 here
# (EPOCHREALTIME).

# shellcheck disable=SC2016
# Single quotes are load-bearing: the child shells expand them.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

[[ -n "${EPOCHREALTIME:-}" ]] || test_skip "needs Bash 5 (EPOCHREALTIME) to time the prompts"
[[ -x "$ROOT/gpy-agent/target/debug/gpy" ]] || test_skip "gpy is not built (cargo build --bins)"
RUNNER="$ROOT/gpy-agent/src/prompt/g_runner.bash"

RUNS="${GPY_TEST_G_LATENCY_RUNS:-20}"
if [[ -n "${CI:-}" ]]; then
    STRICT="${GPY_STRICT_DEADLINE:-0}"
else
    STRICT="${GPY_STRICT_DEADLINE:-1}"
fi
# The scripts must start and finish inside T for the fast segment to show, and
# a prompt must come back within T + HANG_MS. Where strict timing is off (shared
# CI runners, macOS above all: load 10 and prompts 400 ms late were seen) T is
# wider and the hang slack looser. A missed deadline still stands out: the hung
# scripts sleep 9 s, so the prompt would take far longer than any slack here.
if [[ "$STRICT" == 1 ]]; then
    T_MS=100
    HANG_MS=500
else
    T_MS=300
    HANG_MS=2000
fi
GRACE_MS=50
MARGIN_MS=30
BOUND_US=$(((T_MS + GRACE_MS + MARGIN_MS) * 1000))
HANG_US=$(((T_MS + HANG_MS) * 1000))

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-deadline.XXXXXX")"
T="$(cd "$T" && pwd -P)"
# Marks the sleeps of the hung scripts, so the survivor check sees only ours.
MARK="9.$$"
cleanup() {
    pkill -KILL -f "sleep $MARK" 2>/dev/null
    rm -rf "$T"
}
trap cleanup EXIT

while IFS='=' read -r env_name _; do
    case "$env_name" in
        GPY_*) unset "$env_name" ;;
    esac
done < <(env)
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$T"
export GPY_AGENT_SOCKET_PATH="$T/missing.sock" GPY_NERD_FONT=none
export HOME="$T/home" TMPDIR="$T/tmp" XDG_RUNTIME_DIR="$T/run"
export PATH="$ROOT/gpy-agent/target/debug:$PATH"
PLUG="$XDG_CONFIG_HOME/gpy/plugins/lat"
mkdir -p "$HOME" "$TMPDIR" "$XDG_RUNTIME_DIR" "$PLUG/bin" "$PLUG/segments"
chmod 700 "$XDG_RUNTIME_DIR"
chmod -R go-w "$T"

printf '#!/bin/sh\necho fast-ok\n' >"$PLUG/bin/fast.sh"
printf '#!/bin/sh\necho partial-hung\nsleep %s\n' "$MARK" >"$PLUG/bin/hung.sh"
printf '#!/bin/sh\ntrap "" TERM\necho partial-ign\nsleep %s\n' "$MARK" >"$PLUG/bin/ign.sh"
# Mixed-timeout case: notes its PID, then a TERM in a file, and keeps going.
printf '#!/bin/sh\necho $$ >"%s/mix.pid"\ntrap "echo term >\\"%s/mix.term\\"" TERM\nwhile :; do sleep %s; done\n' \
    "$T" "$T" "$MARK" >"$PLUG/bin/catch.sh"
chmod 755 "$PLUG"/bin/*.sh

script_sh=/bin/sh
[[ -x /private/var/select/sh ]] && script_sh=/bin/bash

# The three segments as a plugin, the only segments enabled.
cat >"$PLUG/plugin.toml" <<'EOF'
id = "lat"
name = "Latency"
version = "1.0.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["lat_fast", "lat_hung", "lat_ign"]
description = "Script segments for the deadline test"
EOF
for name in fast hung ign; do
    printf 'description = "%s"\ncommand = "bin/%s.sh"\ninputs = []\nttl_secs = 0\ntimeout_ms = %s\n' \
        "$name" "$name" "$T_MS" >"$PLUG/segments/lat_$name.toml"
done
printf '\n[ui]\nenabled_segments = ["lat_fast", "lat_hung", "lat_ign"]\n' >>"$XDG_CONFIG_HOME/gpy/config.toml"

# Child body: ROOT RUNS. One untimed due prompt first (the runner Bash is
# resolved there), then RUNS timed ones. Before each, the `.reload` flag
# (written without a fork, outside the timed window) makes the prompt due and
# resets the circuit breaker that two timeouts in a row would trip. E carries
# 1/0 per segment: fast, hung, ign shown.
cat >"$T/child.bash" <<'EOF'
source "$1/bash/gpy.bash" >/dev/null 2>&1 || exit 1
flag="$XDG_RUNTIME_DIR/gpy/shells/$$.reload"
__gpy_precmd 2>/dev/null
for ((i = 0; i < $2; i++)); do
    : >"$flag"
    printf 'S\n'
    __gpy_precmd 2>/dev/null
    shown=""
    for t in fast-ok partial-hung partial-ign; do
        if [[ $PS1 == *"$t"* ]]; then shown+=1; else shown+=0; fi
    done
    printf 'E %s\n' "$shown"
done
EOF

load1() {
    if [[ -r /proc/loadavg ]]; then
        cut -d' ' -f1 /proc/loadavg
    else
        sysctl -n vm.loadavg 2>/dev/null | awk '{ print $2 }'
    fi
}

# pgrep -c is not on macOS; the [s] keeps grep from matching itself.
# The count is polled for up to 3 s: a killed script can take a moment to leave
# the process table on a loaded runner, while a leaked one sleeps for 9 s.
# shellcheck disable=SC2009
survivors() {
    local n=0
    for _ in {1..30}; do
        n=$(ps -axo command= | grep -c "[s]leep $MARK")
        [[ "$n" == 0 ]] && break
        sleep 0.1
    done
    echo "$n"
}

# run_shell LABEL CMD...: RUNS due prompts in CMD, timed from here.
run_shell() {
    local label="$1" line now t0="" bad="" load_before
    shift
    local -a samples=()
    load_before="$(load1)"
    while IFS= read -r line; do
        now=${EPOCHREALTIME/[.,]/}
        case "$line" in
            S) t0=$now ;;
            "E "*)
                [[ -n "$t0" ]] || continue
                samples+=($((10#$now - 10#$t0)))
                [[ "${line#E }" == 100 ]] || bad+="${line#E } "
                t0=""
                ;;
        esac
    done < <("$@" 2>/dev/null)

    if [[ ${#samples[@]} -ne "$RUNS" ]]; then
        fail "$label: $RUNS due prompts expected, ${#samples[@]} measured"
        return
    fi
    local -a sorted
    mapfile -t sorted < <(printf '%s\n' "${samples[@]}" | sort -n)
    # Nearest rank: the ceil(0.99 * n)-th smallest sample.
    local rank=$(((99 * RUNS + 99) / 100))
    local p99=${sorted[rank - 1]} p50=${sorted[(RUNS - 1) / 2]} min=${sorted[0]} max=${sorted[RUNS - 1]}
    local stats
    stats="min $((min / 1000)).$(printf '%03d' $((min % 1000))) ms, p50 $((p50 / 1000)) ms, p99 $((p99 / 1000)).$(printf '%03d' $((p99 % 1000))) ms, max $((max / 1000)) ms, n=$RUNS, load1 ${load_before:-?}->$(load1)"
    if ((p99 <= BOUND_US)); then
        pass "$label: p99 within T+${GRACE_MS}+${MARGIN_MS} = $((BOUND_US / 1000)) ms ($stats)"
    elif [[ "$STRICT" == 1 ]]; then
        fail "$label: p99 over T+${GRACE_MS}+${MARGIN_MS} = $((BOUND_US / 1000)) ms ($stats)"
    else
        printf 'NOTE: %s: p99 over T+%s+%s = %s ms, not enforced (GPY_STRICT_DEADLINE=0) (%s)\n' \
            "$label" "$GRACE_MS" "$MARGIN_MS" "$((BOUND_US / 1000))" "$stats"
    fi
    if ((max <= HANG_US)); then
        pass "$label: every prompt within T+${HANG_MS} = $((HANG_US / 1000)) ms (no hang)"
    else
        fail "$label: a prompt took over T+${HANG_MS} = $((HANG_US / 1000)) ms, the deadline was missed ($stats)"
    fi
    if ((min >= T_MS * 1000)); then
        pass "$label: no prompt before T = $T_MS ms (the scripts ran to the deadline)"
    else
        fail "$label: a prompt came back before T = $T_MS ms ($stats)"
    fi
    if [[ -z "$bad" ]]; then
        pass "$label: the fast segment shows, both hung ones are hidden"
    else
        fail "$label: shown fast/hung/ign expected 100, got: $bad"
    fi
    sleep 0.2
    local left
    left="$(survivors)"
    if [[ "$left" == 0 ]]; then
        pass "$label: no script process survives"
    else
        fail "$label: $left script process(es) survived"
        pkill -KILL -f "sleep $MARK" 2>/dev/null
    fi
}

bash32=""
if [[ -x /bin/bash ]] && /bin/bash -c '((BASH_VERSINFO[0] < 4))' 2>/dev/null; then
    bash32=/bin/bash
fi

mkdir -p "$XDG_RUNTIME_DIR/gpy/shells"
run_shell "Bash $BASH_VERSION" "$BASH" --noprofile --norc "$T/child.bash" "$ROOT" "$RUNS"
if [[ -x /bin/bash && "$(/bin/bash -c 'echo $BASH_VERSION')" != "$BASH_VERSION" ]]; then
    run_shell "Bash $(/bin/bash -c 'echo $BASH_VERSION')" /bin/bash --noprofile --norc "$T/child.bash" "$ROOT" "$RUNS"
fi

# run_mixed LABEL BASH FIRST: one runner batch in BASH with the 100 ms
# catch.sh and the 500 ms hung.sh, FIRST (short or long) given first. Polls
# without a fork (a read on a FIFO nobody writes paces it) for the TERM note
# and the death of catch.sh, both timed from the start of the batch child.
MIX_SHORT_MS=100
MIX_LONG_MS=500
MIX_ATTEMPTS=3
mkfifo "$T/tick"
exec {tick}<>"$T/tick"
run_mixed() {
    local label="$1" bash="$2" first="$3" attempt="$4" t0 now tt="" tk="" pid="" bp out left
    local -a segs
    local -a short=(mix_short "$MIX_SHORT_MS" "" "$PLUG/bin/catch.sh" "" 0)
    local -a long=(mix_long "$MIX_LONG_MS" "" "$PLUG/bin/hung.sh" "" 0)
    if [[ "$first" == short ]]; then
        segs=("${short[@]}" "${long[@]}")
    else
        segs=("${long[@]}" "${short[@]}")
    fi
    label="$label, $first first"
    rm -f "$T/mix.pid" "$T/mix.term" "$T/mix.out"
    t0=${EPOCHREALTIME/[.,]/}
    "$bash" --noprofile --norc -c 'source "$1" && shift && __gpy_g_batch "$@"' gpy \
        "$RUNNER" "$script_sh" "${segs[@]}" >"$T/mix.out" 2>/dev/null &
    bp=$!
    while :; do
        now=${EPOCHREALTIME/[.,]/}
        now=$((10#$now - 10#$t0))
        [[ -z "$tt" && -e "$T/mix.term" ]] && tt=$now
        if [[ -z "$pid" && -s "$T/mix.pid" ]]; then
            read -r pid <"$T/mix.pid"
            [[ "$pid" != *[!0-9]* ]] || pid=""
        fi
        [[ -n "$pid" && -z "$tk" ]] && ! kill -0 "$pid" 2>/dev/null && tk=$now
        [[ -n "$tk" ]] && ! kill -0 "$bp" 2>/dev/null && break
        ((now > 2000000)) && break
        read -r -t 0.002 -u "$tick" _
    done
    wait "$bp"
    out="$(<"$T/mix.out")"
    # A serial reader misses the long deadline every time; a stalled shared runner
    # (macOS CI saw TERM or KILL at 506-577 ms) does not. Retry before failing.
    if ((attempt < MIX_ATTEMPTS && (${tt:-0} >= MIX_LONG_MS * 1000 || ${tk:-0} >= MIX_LONG_MS * 1000))); then
        printf 'NOTE: %s: TERM at %s ms, KILL at %s ms, past the %s ms deadline; retrying (attempt %s of %s)\n' \
            "$label" "$((${tt:-0} / 1000))" "$((${tk:-0} / 1000))" "$MIX_LONG_MS" "$attempt" "$MIX_ATTEMPTS"
        pkill -KILL -f "sleep $MARK" 2>/dev/null
        return 1
    fi
    if [[ -z "$tt" ]]; then
        fail "$label: the ${MIX_SHORT_MS} ms segment never ran its TERM handler (no grace before KILL)"
    elif ((tt < MIX_SHORT_MS * 1000 || tt >= MIX_LONG_MS * 1000)); then
        fail "$label: the ${MIX_SHORT_MS} ms segment got TERM at $((tt / 1000)) ms, not between its own deadline and the ${MIX_LONG_MS} ms one"
    elif ((tt <= (MIX_SHORT_MS + 150) * 1000)); then
        pass "$label: the ${MIX_SHORT_MS} ms segment got TERM at $((tt / 1000)) ms, by its own deadline + 150 ms"
    elif [[ "$STRICT" == 1 ]]; then
        fail "$label: the ${MIX_SHORT_MS} ms segment got TERM at $((tt / 1000)) ms, not by its own deadline + 150 ms"
    else
        printf 'NOTE: %s: TERM at %s ms, over its own deadline + 150 ms, not enforced (GPY_STRICT_DEADLINE=0)\n' \
            "$label" "$((tt / 1000))"
    fi
    if [[ -z "$tk" ]]; then
        fail "$label: the ${MIX_SHORT_MS} ms segment was not killed"
    elif ((tk >= MIX_LONG_MS * 1000)); then
        fail "$label: the ${MIX_SHORT_MS} ms segment was killed at $((tk / 1000)) ms, after the ${MIX_LONG_MS} ms deadline"
    elif [[ -n "$tt" ]]; then
        left=$((tk - tt))
        if ((left <= (GRACE_MS + 100) * 1000)); then
            pass "$label: KILL came $((left / 1000)) ms after TERM"
        elif [[ "$STRICT" == 1 ]]; then
            fail "$label: KILL came $((left / 1000)) ms after TERM, not within the ${GRACE_MS} ms grace + 100 ms"
        else
            printf 'NOTE: %s: KILL came %s ms after TERM, over the %s ms grace + 100 ms, not enforced (GPY_STRICT_DEADLINE=0)\n' \
                "$label" "$((left / 1000))" "$GRACE_MS"
        fi
        if ((left >= 25000)); then
            pass "$label: the grace between TERM and KILL was seen ($((left / 1000)) ms)"
        elif [[ "$STRICT" == 1 ]]; then
            fail "$label: only $((left / 1000)) ms between TERM and KILL, the grace is ${GRACE_MS} ms"
        else
            printf 'NOTE: %s: only %s ms between TERM and KILL, not enforced (GPY_STRICT_DEADLINE=0)\n' \
                "$label" "$((left / 1000))"
        fi
    fi
    if [[ "$out" == *mix_short$'\037'timeout$'\037'* && "$out" == *mix_long$'\037'timeout$'\037'* ]]; then
        pass "$label: both segments report a timeout"
    else
        fail "$label: both segments should report a timeout, got: ${out//[$'\001'-$'\037']/|}"
    fi
    sleep 0.2
    left="$(survivors)"
    if [[ "$left" == 0 ]]; then
        pass "$label: no script process survives"
    else
        fail "$label: $left script process(es) survived"
        pkill -KILL -f "sleep $MARK" 2>/dev/null
    fi
    return 0
}

for first in long short; do
    for attempt in $(seq 1 "$MIX_ATTEMPTS"); do
        run_mixed "Mixed timeouts, Bash $BASH_VERSION" "$BASH" "$first" "$attempt" && break
    done
    if [[ -n "$bash32" && "$("$bash32" -c 'echo $BASH_VERSION')" != "$BASH_VERSION" ]]; then
        for attempt in $(seq 1 "$MIX_ATTEMPTS"); do
            run_mixed "Mixed timeouts, Bash $("$bash32" -c 'echo $BASH_VERSION')" "$bash32" "$first" "$attempt" && break
        done
    fi
done
exec {tick}<&-

test_finish "script segment deadline: no hang, p99 within T + grace + 30 ms where strict"
