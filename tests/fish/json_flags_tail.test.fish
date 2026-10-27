#!/usr/bin/env fish
# tests/fish/json_flags_tail.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Contract test for the shared is_last/is_first/prev_bg JSON tail.
#
# Every IPC request builder in fish/core/ipc.fish (__gpy_build_data_payload,
# __gpy_request_duration, __gpy_request_character, __gpy_request_hostname,
# __gpy_request_username) builds its flags tail through __gpy_json_flags_tail
# and the dispatch loop's single "true"/"" convention. This test:
#
#   1. Exercises __gpy_json_flags_tail directly against every row of
#      tests/fixtures/json_flags_tail_vectors.tsv, the vector file the Bash
#      and Zsh tests read too.
#   2. Pins the exact JSON each of the five IPC builders produces by
#      intercepting __gpy_ipc_send and asserting the request string for
#      several flag combinations.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish

# --- 1. __gpy_json_flags_tail: every row of the shared vector file
# tests/fixtures/json_flags_tail_vectors.tsv (also read by the Bash and Zsh
# tests; see its header for the format).

# Undo the fixture's mini escape scheme: `\e` is the empty string; otherwise
# backslash first, then \r/\t/\n. Real newlines are only ever emitted by
# `printf`, never captured by `(...)`, which would drop a trailing one; callers
# capture with `string collect -Na` so every byte survives.
function unescape_value --argument-names raw
    test "$raw" = '\e'; and return
    set -l s (string replace -a -- '\\\\' \x01 "$raw")
    set s (string replace -a -- '\r' \r "$s")
    set s (string replace -a -- '\t' \t "$s")
    set -l parts (string split -- '\n' "$s")
    printf '%s' (string replace -a -- \x01 '\\' "$parts[1]")
    for part in $parts[2..-1]
        printf '\n%s' (string replace -a -- \x01 '\\' "$part")
    end
end

set -l fixture "$repo_root/tests/fixtures/json_flags_tail_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end
set -l vector_count 0
while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue
    test (count $cols) -lt 4; and continue

    set vector_count (math $vector_count + 1)
    set -l is_last (unescape_value "$cols[1]" | string collect -Na)
    set -l is_first (unescape_value "$cols[2]" | string collect -Na)
    set -l prev_bg (unescape_value "$cols[3]" | string collect -Na)
    set -l expected "$cols[4]"
    test "$expected" = '\e'; and set expected ''
    check "tail vector $vector_count ($cols[1] $cols[2] $cols[3])" "$expected" \
        (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg" | string collect)
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

# --- 2. Payload byte-equality: intercept __gpy_ipc_send and assert the
# exact JSON each builder sends, per flag combination.
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

check "build_data_payload: not-last, not-first, no prev_bg" \
    '{"op":"git","cwd":"/tmp/proj","format":"ansi"}' \
    (__gpy_build_data_payload git /tmp/proj ansi "" "" "")

check "build_data_payload: last, first, prev_bg" \
    '{"op":"git","cwd":"/tmp/proj","format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue"}' \
    (__gpy_build_data_payload git /tmp/proj ansi true blue true)

test_finish "json_flags_tail tests"
