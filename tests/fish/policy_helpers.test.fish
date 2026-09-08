#!/usr/bin/env fish
# tests/fish/policy_helpers.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Unit tests for the pure policy helpers extracted by #612:
#   __gpy_breaker_gate, __gpy_breaker_record_failure (fish/core/util.fish)
#   __gpy_cache_freshness, __gpy_throttle_elapsed (fish/core/util.fish)
#   __gpy_cache_status_stale, __gpy_cache_status_variant (fish/core/util.fish)
#   __gpy_uint_or_default (fish/core/constants.fish)
#
# Each helper is a pure function of its arguments: no fork, no clock read, no
# filesystem access, no globals. This file sources ONLY constants.fish and
# util.fish -- no socket, no clock, no filesystem -- and must FAIL on a tree
# without these functions defined.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/util.fish

set -g pass_count 0
set -g fail_count 0

function check --argument-names label expected actual
    if test "$actual" = "$expected"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected [$expected], got [$actual])"
    end
end

function check_status --argument-names label expected_status actual_status
    if test "$actual_status" -eq "$expected_status"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected status $expected_status, got $actual_status)"
    end
end

# --- __gpy_breaker_gate backoff_until now -> allow|deny ---

check "gate: backoff in the future denies" \
    deny (__gpy_breaker_gate 200 100)

check "gate: backoff in the past allows" \
    allow (__gpy_breaker_gate 50 100)

check "gate: backoff equal to now allows (strictly-after only)" \
    allow (__gpy_breaker_gate 100 100)

check "gate: empty now allows (date fork failed)" \
    allow (__gpy_breaker_gate 200 "")

check "gate: empty backoff_until allows" \
    allow (__gpy_breaker_gate "" 100)

check "gate: non-numeric backoff_until allows" \
    allow (__gpy_breaker_gate abc 100)

check "gate: non-numeric now allows" \
    allow (__gpy_breaker_gate 200 abc)

check "gate: backoff_until zero (breaker closed) allows" \
    allow (__gpy_breaker_gate 0 100)

# --- __gpy_breaker_record_failure failure_count now threshold backoff_secs
#     -> "<new_count> <new_until>" ---

check "record: below threshold just increments, until stays 0" \
    "1 0" (__gpy_breaker_record_failure 0 1000 3 60)

check "record: one below threshold" \
    "2 0" (__gpy_breaker_record_failure 1 1000 3 60)

check "record: reaching threshold opens the breaker and resets the count" \
    "0 1060" (__gpy_breaker_record_failure 2 1000 3 60)

check "record: past threshold still opens (count already inconsistent)" \
    "0 1060" (__gpy_breaker_record_failure 5 1000 3 60)

check "record: empty failure_count treated as 0" \
    "1 0" (__gpy_breaker_record_failure "" 1000 3 60)

check "record: opening with empty now treats it as 0" \
    "0 60" (__gpy_breaker_record_failure 2 "" 3 60)

# --- __gpy_cache_freshness now mtime ttl -> fresh|stale ---

check "freshness: well within TTL is fresh" \
    fresh (__gpy_cache_freshness 100 99 5)

check "freshness: exactly at TTL boundary is stale (>=)" \
    stale (__gpy_cache_freshness 105 100 5)

check "freshness: one second past TTL boundary is stale" \
    stale (__gpy_cache_freshness 106 100 5)

check "freshness: one second under TTL boundary is fresh" \
    fresh (__gpy_cache_freshness 104 100 5)

check "freshness: empty now is fresh (unknown treated as fresh)" \
    fresh (__gpy_cache_freshness "" 100 5)

check "freshness: empty mtime is fresh" \
    fresh (__gpy_cache_freshness 100 "" 5)

check "freshness: empty ttl is fresh" \
    fresh (__gpy_cache_freshness 100 90 "")

check "freshness: non-numeric operand is fresh" \
    fresh (__gpy_cache_freshness abc 100 5)

# --- __gpy_throttle_elapsed last_ms now_ms interval_ms -> exit 0/1 ---

__gpy_throttle_elapsed 1000 1400 500
check_status "throttle: under interval does not elapse" 1 $status

__gpy_throttle_elapsed 1000 1500 500
check_status "throttle: exactly at interval elapses (>=)" 0 $status

__gpy_throttle_elapsed 1000 1600 500
check_status "throttle: past interval elapses" 0 $status

__gpy_throttle_elapsed "" 1000 500
check_status "throttle: empty last_ms treated as 0, elapses" 0 $status

__gpy_throttle_elapsed "" 100 500000
check_status "throttle: empty last_ms with huge interval does not elapse" 1 $status

# --- __gpy_cache_status_stale / __gpy_cache_status_variant <code> ---

for code_pair in "0 0 0" "1 0 0" "2 1 0" "4 0 1" "6 1 1"
    set -l parts (string split ' ' -- $code_pair)
    set -l code $parts[1]
    set -l expect_stale $parts[2]
    set -l expect_variant $parts[3]

    __gpy_cache_status_stale $code
    set -l got_stale 0
    test $status -eq 0; and set got_stale 1
    check "cache_status_stale($code)" "$expect_stale" "$got_stale"

    __gpy_cache_status_variant $code
    set -l got_variant 0
    test $status -eq 0; and set got_variant 1
    check "cache_status_variant($code)" "$expect_variant" "$got_variant"
end

# --- __gpy_uint_or_default value default -> value|default ---

check "uint_or_default: numeric value is kept" \
    30 (__gpy_uint_or_default 30 5)

check "uint_or_default: zero is kept (valid non-negative integer)" \
    0 (__gpy_uint_or_default 0 5)

check "uint_or_default: empty value falls back to default" \
    5 (__gpy_uint_or_default "" 5)

check "uint_or_default: non-numeric value falls back to default" \
    5 (__gpy_uint_or_default abc 5)

check "uint_or_default: negative value falls back to default" \
    5 (__gpy_uint_or_default -3 5)

check "uint_or_default: whitespace-padded value falls back to default" \
    5 (__gpy_uint_or_default " 3" 5)

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count policy helper tests passed"
