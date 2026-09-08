#!/usr/bin/env fish
# tests/fish/json_escape_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #614: shared JSON-escape test vector file
# (tests/fixtures/json_escape_vectors.tsv), run by all three shells, so
# __gpy_json_escape (Fish), __gpy_escape_json (Bash), and __gpy_escape_json
# (Zsh) can never silently drift apart again. Fish's implementation
# (fish/core/ipc.fish) already escapes \n/\r/\t; this test is what proves it
# against the same vectors bash/zsh check.

set -l script_dir (path dirname (status --current-filename))
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

# Undo the fixture's input-column mini escape scheme (see its own header
# comment): backslash first, then control chars. Uses a sentinel byte to
# protect literal backslash pairs from being re-touched by the later
# \n/\r/\t substitutions. Every step pipes through `string collect` -- a
# result containing a real embedded newline would otherwise be split into
# multiple list elements the instant it crosses a `(...)` command
# substitution boundary, silently dropping the newline when later
# reassembled as a plain string.
function unescape_input --argument-names raw
    set -l sentinel \x01
    set -l s (string replace -a -- '\\\\' "$sentinel" "$raw" | string collect)
    set s (string replace -a -- '\n' \n "$s" | string collect)
    set s (string replace -a -- '\r' \r "$s" | string collect)
    set s (string replace -a -- '\t' \t "$s" | string collect)
    set s (string replace -a -- "$sentinel" '\\' "$s" | string collect)
    printf '%s' "$s"
end

set -l fixture "$repo_root/tests/fixtures/json_escape_vectors.tsv"
set -l vector_count 0

# `-d \t` splits ONLY on tab (fish's default `read` also splits on spaces,
# which would wrongly break apart a column like "plain text no special
# chars" that contains no tab at all).
while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    set -l first_char (string sub -l 1 -- "$cols[1]")
    test "$first_char" = '#'; and continue
    test (count $cols) -lt 2; and continue

    set vector_count (math $vector_count + 1)
    set -l input_col $cols[1]
    set -l expected_col $cols[2]
    # `string collect`: unescape_input's result may contain a real embedded
    # newline (for the \n vectors) -- see its own comment for why every
    # `(...)` boundary a raw newline crosses needs this.
    set -l real_input (unescape_input "$input_col" | string collect)
    set -l actual (__gpy_json_escape "$real_input")
    check "vector $vector_count ($input_col)" "$expected_col" "$actual"
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    set fail_count (math $fail_count + 1)
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count JSON-escape vector tests passed"
