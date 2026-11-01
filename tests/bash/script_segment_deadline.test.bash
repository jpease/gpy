#!/usr/bin/env bash
# tests/bash/script_segment_deadline.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The script (G) segment deadline in every shell's real adapter (ADR-0011
# item 6): a due prompt whose batch holds a hung script, one that ignores
# TERM and keeps its stdout, and a fast one, all with timeout_ms T, never
# draws before T (the scripts really ran), the fast segment still shows, both
# hung ones are hidden, and no script process survives. A latency test, not a
# benchmark: scripts/ci-bench.sh measures cost.
#
# Mixed timeouts: one runner batch (bash/core/g_runner.bash, which Zsh and
# Fish run in a Bash child) holds a 500 ms hung script and a 100 ms script
# that catches TERM and keeps running, in both orders. The short one must get
# TERM by its own deadline (not the long one), then the grace (its TERM
# handler runs) and KILL soon after. Hang-type bounds, always enforced: TERM
# and KILL both before the long segment's deadline, which a reader waiting on
# the long segment misses. Strict bounds: TERM within T + 150 ms, KILL within
# the 50 ms grace + 100 ms of it, and 25 ms of the grace seen between the two.
#
# Two latency bounds. Strict: p99 (nearest rank over RUNS due prompts per
# shell) within T + 50 ms grace + 30 ms, the quiet-host acceptance of
# ADR-0011 Spike 2. Hang: every prompt within T + 500 ms, which only a missed
# deadline breaks. GPY_STRICT_DEADLINE=1 enforces both; 0 enforces the hang
# bound and only reports strict. The default is 1, or 0 under CI: shared
# runners (macOS above all) are too noisy for strict. The advisory
# strict step of the Performance budgets job in pr-gate.yml runs this test with
# GPY_STRICT_DEADLINE=1.
#
# Shells: this Bash, /bin/bash when it is another version (macOS 3.2: the
# timer-group path), Zsh and Fish with their default runner child and, where
# /bin/bash is Bash < 4, with /bin/bash as that child. Each child prints S
# right before a due prompt and E after it; this Bash stamps both lines as
# they arrive, so no clock is needed inside the child.
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

RUNS="${GPY_TEST_G_LATENCY_RUNS:-20}"
T_MS=100
GRACE_MS=50
MARGIN_MS=30
BOUND_US=$(((T_MS + GRACE_MS + MARGIN_MS) * 1000))
HANG_MS=500
HANG_US=$(((T_MS + HANG_MS) * 1000))
if [[ -n "${CI:-}" ]]; then
    STRICT="${GPY_STRICT_DEADLINE:-0}"
else
    STRICT="${GPY_STRICT_DEADLINE:-1}"
fi

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-deadline.XXXXXX")"
T="$(cd "$T" && pwd -P)"
# Marks the sleeps of the hung scripts, so the survivor check sees only ours.
MARK="9.$$"
cleanup() {
    pkill -KILL -f "sleep $MARK" 2>/dev/null
    rm -rf "$T"
}
trap cleanup EXIT

while IFS='=' read -r __gpy_env_name _; do
    case "$__gpy_env_name" in
        GPY_*) unset "$__gpy_env_name" ;;
    esac
done < <(env)
source "$ROOT/tests/lib/supervisor_off.bash" "$T"
export GPY_AGENT_SOCKET_PATH="$T/missing.sock"
export HOME="$T/home" TMPDIR="$T/tmp" XDG_RUNTIME_DIR="$T/run"
mkdir -p "$HOME" "$TMPDIR" "$XDG_RUNTIME_DIR" "$T/plug/bin" "$T/stub"
chmod 700 "$XDG_RUNTIME_DIR"
chmod 755 "$T/plug" "$T/plug/bin"

printf '#!/bin/sh\necho fast-ok\n' >"$T/plug/bin/fast.sh"
printf '#!/bin/sh\necho partial\nsleep %s\n' "$MARK" >"$T/plug/bin/hung.sh"
printf '#!/bin/sh\ntrap "" TERM\necho partial\nsleep %s\n' "$MARK" >"$T/plug/bin/ign.sh"
# Mixed-timeout case: notes its PID, then a TERM in a file, and keeps going.
printf '#!/bin/sh\necho $$ >"%s/mix.pid"\ntrap "echo term >\\"%s/mix.term\\"" TERM\nwhile :; do sleep %s; done\n' \
    "$T" "$T" "$MARK" >"$T/plug/bin/catch.sh"
chmod 644 "$T"/plug/bin/*.sh

# Fish loads script segments only with the agent enabled; this stub makes
# its start (and any oneshot) fail at once instead of reaching a real agent.
printf '#!/bin/sh\nexit 1\n' >"$T/stub/gpy-agent"
chmod 755 "$T/stub/gpy-agent"

script_sh=/bin/sh
[[ -x /private/var/select/sh ]] && script_sh=/bin/bash

# The three segments as the theme export writes them, for shell $1.
write_export() {
    local kind="$1" tok name assign list
    case "$kind" in
        bash) assign='%s="%s"\n' list='%s=(%s)\n' ;;
        zsh) assign='typeset -g %s="%s"\n' list='typeset -g -a %s=(%s)\n' ;;
        fish) assign='set -g %s "%s"\n' list='set -g %s %s\n' ;;
    esac
    {
        # shellcheck disable=SC2059
        printf "$assign" __gpy_script_sh "$script_sh"
        for name in lat-fast lat-hung lat-ign; do
            tok="${name//-/_}"
            # shellcheck disable=SC2059
            {
                printf "$assign" "__gpy_script_${tok}_name" "$name"
                printf "$assign" "__gpy_script_${tok}_root" "$T/plug"
                printf "$assign" "__gpy_script_${tok}_command" "$T/plug/bin/${name#lat-}.sh"
                printf "$assign" "__gpy_script_${tok}_reqcommand" ""
                printf "$assign" "__gpy_script_${tok}_ttl" 0
                printf "$assign" "__gpy_script_${tok}_timeout" "$T_MS"
                printf "$assign" "__gpy_script_${tok}_scope" session
                printf "$assign" "__gpy_script_${tok}_key" "4:test5:1.0.0${#name}:$name"
                for f in inputs env opts reqfiles reqfolders reqexts reqenv; do
                    printf "$list" "__gpy_script_${tok}_$f" ""
                done
            }
        done
        # shellcheck disable=SC2059
        printf "$list" __gpy_script_segments '"lat_fast" "lat_hung" "lat_ign"'
        # shellcheck disable=SC2059  # $assign is the per-shell format
        case "$kind" in
            fish) printf 'set -g __enabled_segments lat-fast lat-hung lat-ign\n' ;;
            *) printf "$assign" __enabled_segments "lat-fast lat-hung lat-ign" ;;
        esac
    } >"$XDG_CACHE_HOME/gpy/theme-export.$kind"
}
mkdir -p "$XDG_CACHE_HOME/gpy"
write_export bash
write_export zsh
write_export fish

# Child bodies: ROOT RUNS [RUNNER]. One untimed due prompt first (the runner
# child is resolved there), then RUNS timed ones. Each due prompt follows a
# theme reload, which resets the circuit breaker that two timeouts in a row
# would trip. E carries 1/0 per segment: fast, hung, ign shown.
cat >"$T/child.bash" <<'EOF'
source "$1/bash/gpy.bash" >/dev/null 2>&1 || exit 1
# Into $shown without a fork, which would land inside the timed window.
shown_now() {
    local t
    shown=""
    for t in lat_fast lat_hung lat_ign; do
        if __gpy_g_visible "$t"; then shown+=1; else shown+=0; fi
    done
}
__gpy_load_theme >/dev/null 2>&1
__gpy_precmd
for ((i = 0; i < $2; i++)); do
    __gpy_load_theme >/dev/null 2>&1
    printf 'S\n'
    __gpy_precmd
    shown_now
    printf 'E %s\n' "$shown"
done
EOF
cat >"$T/child.zsh" <<'EOF'
source "$1/zsh/gpy.zsh" >/dev/null 2>&1 || exit 1
# A zsh script runs preexec before each top-level line; precmd is called here.
preexec_functions=()
if [[ -n ${3-} ]]; then
    __gpy_g_bash=$3 __gpy_g_bash_resolved=1
fi
shown_now() {
    local t
    shown=""
    for t in lat_fast lat_hung lat_ign; do
        [[ -n ${__gpy_g_text[$t]-} ]] && shown+=1 || shown+=0
    done
}
integer i
__gpy_reload_config >/dev/null 2>&1
for f in $precmd_functions; do $f; done
for ((i = 0; i < $2; i++)); do
    __gpy_reload_config >/dev/null 2>&1
    print S
    for f in $precmd_functions; do $f; done
    shown_now
    print -r -- "E $shown"
done
EOF
cat >"$T/child.fish" <<'EOF'
source $argv[1]/fish/core/init.fish >/dev/null 2>&1; or exit 1
if test -n "$argv[3]"
    set -g __gpy_g_bash $argv[3]
    set -l runner $__gpy_core_dir/g_runner.bash
    test -r $runner; or set runner (path normalize $__gpy_core_dir/../../bash/core/g_runner.bash)
    set -g __gpy_g_src (string collect <$runner)
    set -g __gpy_g_runner_state ready
end
function shown
    set -l v ""
    for t in lat_fast lat_hung lat_ign
        set -l n __gpy_g_t_{$t}_text
        if test -n "$$n"
            set v {$v}1
        else
            set v {$v}0
        end
    end
    printf '%s' $v
end
function due
    __gpy_apply_theme_export >/dev/null 2>&1
    emit fish_postexec true
    emit fish_prompt
    fish_prompt >/dev/null 2>&1
end
due
for i in (seq $argv[2])
    __gpy_apply_theme_export >/dev/null 2>&1
    echo S
    emit fish_postexec true
    emit fish_prompt
    fish_prompt >/dev/null 2>&1
    echo "E "(shown)
end
EOF

load1() {
    if [[ -r /proc/loadavg ]]; then
        cut -d' ' -f1 /proc/loadavg
    else
        sysctl -n vm.loadavg 2>/dev/null | awk '{ print $2 }'
    fi
}

# pgrep -c is not on macOS; the [s] keeps grep from matching itself.
# shellcheck disable=SC2009
survivors() {
    ps -axo command= | grep -c "[s]leep $MARK"
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

run_shell "Bash $BASH_VERSION" "$BASH" --noprofile --norc "$T/child.bash" "$ROOT" "$RUNS"
if [[ -n "$bash32" && "$("$bash32" -c 'echo $BASH_VERSION')" != "$BASH_VERSION" ]]; then
    run_shell "Bash $("$bash32" -c 'echo $BASH_VERSION')" "$bash32" --noprofile --norc "$T/child.bash" "$ROOT" "$RUNS"
fi

if command -v zsh >/dev/null 2>&1; then
    run_shell "Zsh, default runner child" zsh -f "$T/child.zsh" "$ROOT" "$RUNS"
    [[ -n "$bash32" ]] && run_shell "Zsh, $bash32 runner child" zsh -f "$T/child.zsh" "$ROOT" "$RUNS" "$bash32"
else
    test_skip_part "zsh not installed"
fi

if command -v fish >/dev/null 2>&1; then
    fish_env=(env "PATH=$T/stub:$PATH" GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0)
    run_shell "Fish, default runner child" "${fish_env[@]}" fish --no-config "$T/child.fish" "$ROOT" "$RUNS"
    [[ -n "$bash32" ]] &&
        run_shell "Fish, $bash32 runner child" "${fish_env[@]}" fish --no-config "$T/child.fish" "$ROOT" "$RUNS" "$bash32"
else
    test_skip_part "fish not installed"
fi

# run_mixed LABEL BASH FIRST: one runner batch in BASH with the 100 ms
# catch.sh and the 500 ms hung.sh, FIRST (short or long) given first. Polls
# without a fork (a read on a FIFO nobody writes paces it) for the TERM note
# and the death of catch.sh, both timed from the start of the batch child.
MIX_SHORT_MS=100
MIX_LONG_MS=500
mkfifo "$T/tick"
exec {tick}<>"$T/tick"
run_mixed() {
    local label="$1" bash="$2" first="$3" t0 now tt="" tk="" pid="" bp out left
    local -a segs
    local -a short=(mix_short "$MIX_SHORT_MS" "" "$T/plug/bin/catch.sh" "" 0)
    local -a long=(mix_long "$MIX_LONG_MS" "" "$T/plug/bin/hung.sh" "" 0)
    if [[ "$first" == short ]]; then
        segs=("${short[@]}" "${long[@]}")
    else
        segs=("${long[@]}" "${short[@]}")
    fi
    label="$label, $first first"
    rm -f "$T/mix.pid" "$T/mix.term" "$T/mix.out"
    t0=${EPOCHREALTIME/[.,]/}
    "$bash" --noprofile --norc -c 'source "$1" && shift && __gpy_g_batch "$@"' gpy \
        "$ROOT/bash/core/g_runner.bash" "$script_sh" "${segs[@]}" >"$T/mix.out" 2>/dev/null &
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
}

for first in long short; do
    run_mixed "Mixed timeouts, Bash $BASH_VERSION" "$BASH" "$first"
    if [[ -n "$bash32" && "$("$bash32" -c 'echo $BASH_VERSION')" != "$BASH_VERSION" ]]; then
        run_mixed "Mixed timeouts, Bash $("$bash32" -c 'echo $BASH_VERSION')" "$bash32" "$first"
    fi
done
exec {tick}<&-

test_finish "script segment deadline: no hang in every shell, p99 within T + grace + 30 ms where strict"
