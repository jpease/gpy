#!/usr/bin/env fish
# tests/fish/json_flags_tail.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/contract test for #613.
#
# Before this issue, is_last/is_first/prev_bg JSON tail-building was hand-
# duplicated across five IPC request builders in fish/core/ipc.fish
# (__gpy_build_data_payload, __gpy_request_duration, __gpy_request_character,
# __gpy_request_hostname, __gpy_request_username), and each segment
# re-converted the dispatch loop's is_last/is_first into whatever local
# convention it felt like using. This test:
#
#   1. Exercises __gpy_json_flags_tail directly across every
#      (is_last, is_first, prev_bg) combination, plus a prev_bg that needs
#      JSON escaping.
#   2. Proves the five refactored IPC builders produce byte-identical JSON by
#      intercepting __gpy_ipc_send and asserting the exact request string for
#      several flag combinations.
#
# Must FAIL on a tree without __gpy_json_flags_tail (function undefined).

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish

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

# --- 1. __gpy_json_flags_tail: all 8 (is_last, is_first, prev_bg) combos ---
# plus a 9th case where prev_bg needs JSON escaping.

check "tail: not-last, not-first, no prev_bg" \
    '' (__gpy_json_flags_tail "" "" "")

check "tail: last, not-first, no prev_bg" \
    ',"is_last":true' (__gpy_json_flags_tail true "" "")

check "tail: not-last, first, no prev_bg" \
    ',"is_first":true' (__gpy_json_flags_tail "" true "")

check "tail: last, first, no prev_bg" \
    ',"is_last":true,"is_first":true' (__gpy_json_flags_tail true true "")

check "tail: not-last, not-first, prev_bg" \
    ',"prev_bg":"blue"' (__gpy_json_flags_tail "" "" blue)

check "tail: last, not-first, prev_bg" \
    ',"is_last":true,"prev_bg":"blue"' (__gpy_json_flags_tail true "" blue)

check "tail: not-last, first, prev_bg" \
    ',"is_first":true,"prev_bg":"blue"' (__gpy_json_flags_tail "" true blue)

check "tail: last, first, prev_bg" \
    ',"is_last":true,"is_first":true,"prev_bg":"blue"' (__gpy_json_flags_tail true true blue)

# 9th case: prev_bg needing escaping (embedded quote + backslash). Backslash
# is escaped first, then the quote, mirroring __gpy_json_escape's own order.
# NOTE: this string is single-quoted, so `\\\\` here is fish's escape for TWO
# literal backslashes (the escaped-JSON form of the one input backslash below).
check "tail: last, prev_bg needing escaping" \
    ',"is_last":true,"prev_bg":"blue\"bg\\\\path"' \
    (__gpy_json_flags_tail true "" 'blue"bg\path')

# --- 2. Payload byte-equality proof: intercept __gpy_ipc_send and assert the
# exact JSON each refactored builder sends, per flag combination. This makes
# the refactor's "byte-identical payloads for the same inputs" claim a
# permanent, executable check rather than a one-off manual diff.
function __gpy_ipc_send
    # Echo the payload verbatim instead of touching a real socket.
    echo $argv[1]
end

check "duration payload: last+first" \
    '{"op":"duration","duration_ms":500,"format":"ansi","is_last":true,"is_first":true}' \
    (__gpy_request_duration 500 true "" true)

check "duration payload: not-last, not-first" \
    '{"op":"duration","duration_ms":0,"format":"ansi"}' \
    (__gpy_request_duration 0 "" "" "")

check "character payload: last, prev_bg" \
    '{"op":"character","success":true,"format":"ansi","is_last":true,"prev_bg":"cyan"}' \
    (__gpy_request_character 1 true cyan)

check "character payload: failure, not-last" \
    '{"op":"character","success":false,"format":"ansi"}' \
    (__gpy_request_character 0 "" "")

check "hostname payload: last, prev_bg" \
    '{"op":"hostname","hostname":"myhost","format":"ansi","is_last":true,"prev_bg":"green"}' \
    (__gpy_request_hostname myhost true green)

check "hostname payload: not-last, no prev_bg" \
    '{"op":"hostname","hostname":"myhost","format":"ansi"}' \
    (__gpy_request_hostname myhost "" "")

check "hostname payload: prev_bg needing escaping" \
    '{"op":"hostname","hostname":"myhost","format":"ansi","is_last":true,"prev_bg":"weird\"bg\\\\path"}' \
    (__gpy_request_hostname myhost true 'weird"bg\path')

check "username payload: last, prev_bg" \
    '{"op":"username","username":"bob","format":"ansi","is_last":true,"prev_bg":"red"}' \
    (__gpy_request_username bob true red)

check "username payload: not-last, no prev_bg" \
    '{"op":"username","username":"bob","format":"ansi"}' \
    (__gpy_request_username bob "" "")

check "build_data_payload: not-last, not-first, no prev_bg" \
    '{"op":"git","cwd":"/tmp/proj","format":"ansi"}' \
    (__gpy_build_data_payload git /tmp/proj ansi "" "" "")

check "build_data_payload: last, first, prev_bg" \
    '{"op":"git","cwd":"/tmp/proj","format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue"}' \
    (__gpy_build_data_payload git /tmp/proj ansi true blue true)

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count json_flags_tail tests passed"
