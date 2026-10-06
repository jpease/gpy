#!/usr/bin/env fish
# tests/fish/cache_key_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #705: __gpy_path_to_cache_key must produce the same Unix key as the agent's
# encoder for every row of tests/fixtures/cache_key_vectors.tsv (also read by
# the Rust, Bash and Zsh tests). See the fixture's header for the format.

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

# Undo the fixture's input-column mini escape scheme (see its header).
function unescape_input --argument-names raw
    set -l sentinel \x01
    set -l s (string replace -a -- '\\\\' "$sentinel" "$raw" | string collect)
    set s (string replace -a -- '\n' \n "$s" | string collect)
    set s (string replace -a -- '\r' \r "$s" | string collect)
    set s (string replace -a -- '\t' \t "$s" | string collect)
    set s (string replace -a -- "$sentinel" '\\' "$s" | string collect)
    printf '%s' "$s"
end

set -l fixture "$repo_root/tests/fixtures/cache_key_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end
set -l vector_count 0

while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    set -l first_char (string sub -l 1 -- "$cols[1]")
    test "$first_char" = '#'; and continue
    test (count $cols) -lt 2; and continue

    set vector_count (math $vector_count + 1)
    set -l input_col $cols[1]
    set -l expected_col $cols[2]
    set -l real_input (unescape_input "$input_col" | string collect)
    set -l actual (__gpy_path_to_cache_key "$real_input")
    check "vector $vector_count ($input_col)" "$expected_col" "$actual"
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count cache-key vector tests passed"
