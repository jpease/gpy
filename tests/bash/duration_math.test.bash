#!/usr/bin/env bash
# tests/bash/duration_math.test.bash
#
# Regression test for #341: the bash duration segment computed
# EPOCHREALTIME-to-milliseconds via an `awk` fork on every prompt
# (bash/core/init.bash, epochrealtime branch). __gpy_epoch_diff_ms replaces
# that with pure-bash integer arithmetic assigned via indirect `printf -v`
# (no fork at all).
#
# This compares __gpy_epoch_diff_ms's output against the *original* awk
# formula across representative EPOCHREALTIME value pairs: same-second
# sub-millisecond deltas, a cross-second boundary, a value with a
# leading-zero fractional part (the octal-literal trap `10#` guards against),
# a value with a leading-zero integer part, and a comma-decimal-separator
# string (confirmed for real: `LC_ALL=de_DE.UTF-8 bash -c 'echo
# $EPOCHREALTIME'` prints e.g. "1783061985,338122" on this machine).
#
# Bash's arithmetic truncates instead of rounding (unlike awk's `%.0f`), so
# results are asserted within +/-1ms rather than requiring exact equality --
# the same tolerance the issue's acceptance criteria specify.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# The theme export re-sets the supervisor flag from config.toml, so disable
# it in a sandbox config; the missing socket stays there too (#836).
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

# old_awk_ms START END -- the exact formula __gpy_precmd used before #341.
old_awk_ms() {
    local start="$1" end="$2"
    awk "BEGIN {printf \"%.0f\", ($end - $start) * 1000}"
}

# assert_within_tolerance LABEL START END TOLERANCE_MS
# Compares __gpy_epoch_diff_ms's result against the old awk formula.
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

# assert_exact_ms LABEL START END EXPECTED_MS -- for cases the old awk formula
# cannot be trusted as a reference (comma-decimal locale strings; awk parses
# only up to the first non-numeric byte, so it silently mis-evaluates a
# comma-separated value instead of computing the real delta).
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

# Leading-zero fractional part (58900us). This is the octal-literal trap:
# without `10#`, bash's $(( )) would treat "058901"/"000001" as octal and
# error on the digits 8/9 in "058901".
assert_within_tolerance "leading-zero fraction (octal trap)" "1000.000001" "1000.058901" 1

# Leading-zero integer part -- synthetic (EPOCHREALTIME's integer half is a
# real epoch second count and would never actually start with 0), but proves
# `10#` on the integer half is not skipped either.
assert_within_tolerance "leading-zero integer part" "098.000100" "098.099900" 1

# Larger, more realistic cross-second delta.
assert_within_tolerance "realistic 1.5s command" "1751500000.123456" "1751500001.654321" 1

echo ""
echo "=== Duration math: comma-decimal locale (verified real bash behavior) ==="
# Real bash under a comma-decimal locale prints EPOCHREALTIME with a comma:
#   LC_ALL=de_DE.UTF-8 bash -c 'echo $EPOCHREALTIME' -> "1783061985,338122"
# The old awk formula cannot be used as a reference here: awk parses a
# numeric string only up to the first non-numeric byte, so "501,000000"
# evaluates as 501 rather than 501.000000, silently producing the wrong
# delta. __gpy_epoch_diff_ms fixes this by splitting on a `[.,]` character
# class instead of a literal ".", so it is checked against a hand-computed
# expected value instead.
assert_exact_ms "comma-locale, 750ms delta" "500,250000" "501,000000" 750

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
