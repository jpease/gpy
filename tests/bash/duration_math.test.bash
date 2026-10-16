#!/usr/bin/env bash
# tests/bash/duration_math.test.bash
#
# __gpy_epoch_diff_ms (pure-bash, no fork) matches the awk formula within +/-1ms

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# Sandbox config disables the supervisor (theme export re-sets the flag).
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-duration-math.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source bash/gpy.bash

if ! declare -f __gpy_epoch_diff_ms &>/dev/null; then
    echo "FAIL: __gpy_epoch_diff_ms not defined"
    exit 1
fi

FAILED=0

# old_awk_ms START END -- the awk reference formula (float subtract, round).
old_awk_ms() {
    local start="$1" end="$2"
    awk "BEGIN {printf \"%.0f\", ($end - $start) * 1000}"
}

# assert_within_tolerance LABEL START END TOLERANCE_MS
assert_within_tolerance() {
    local label="$1" start="$2" end="$3" tolerance="$4"

    local new_ms
    if ! __gpy_epoch_diff_ms "$start" "$end" new_ms; then
        echo "FAIL: [$label] __gpy_epoch_diff_ms rejected ($start -> $end)"
        FAILED=1
        return
    fi

    local old_ms
    old_ms="$(old_awk_ms "$start" "$end")"

    local diff=$(( new_ms - old_ms ))
    (( diff < 0 )) && diff=$(( -diff ))

    if (( diff > tolerance )); then
        echo "FAIL: [$label] new=$new_ms old=$old_ms diff=${diff}ms exceeds tolerance ${tolerance}ms ($start -> $end)"
        FAILED=1
    else
        echo "PASS: [$label] new=${new_ms}ms old=${old_ms}ms diff=${diff}ms ($start -> $end)"
    fi
}

# assert_exact_ms: awk stops at ',' so it can't be the reference for comma decimals.
assert_exact_ms() {
    local label="$1" start="$2" end="$3" expected="$4"

    local new_ms
    if ! __gpy_epoch_diff_ms "$start" "$end" new_ms; then
        echo "FAIL: [$label] __gpy_epoch_diff_ms rejected ($start -> $end)"
        FAILED=1
        return
    fi

    if [[ "$new_ms" != "$expected" ]]; then
        echo "FAIL: [$label] new=$new_ms expected=$expected ($start -> $end)"
        FAILED=1
    else
        echo "PASS: [$label] new=${new_ms}ms matches expected ${expected}ms ($start -> $end)"
    fi
}

echo "=== Duration math: new bash arithmetic vs old awk formula ==="

# Same-second, sub-millisecond delta (500us).
assert_within_tolerance "same-second sub-ms" "1234.000100" "1234.000600" 1

# Cross-second boundary (300us, straddling whole-second rollover).
assert_within_tolerance "cross-second boundary" "1234.999900" "1235.000200" 1

# Leading-zero fraction: without `10#`, $(( )) parses 058901 as octal.
assert_within_tolerance "leading-zero fraction (octal trap)" "1000.000001" "1000.058901" 1

# Leading-zero integer part (synthetic; proves `10#` covers it too).
assert_within_tolerance "leading-zero integer part" "098.000100" "098.099900" 1

assert_within_tolerance "realistic 1.5s command" "1751500000.123456" "1751500001.654321" 1

echo ""
echo "=== Duration math: comma-decimal locale (verified real bash behavior) ==="
# Comma-decimal locale EPOCHREALTIME; checked against a hand-computed value.
assert_exact_ms "comma-locale, 750ms delta" "500,250000" "501,000000" 750

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
