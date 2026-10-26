#!/usr/bin/env fish
# tests/fish/data_request_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_build_data_payload builds the same git/lang request bytes as Bash's and
# Zsh's __gpy_build_data_request for every row of
# tests/fixtures/data_request_vectors.tsv (also read by the Bash and Zsh
# tests). See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish

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

set -l fixture "$repo_root/tests/fixtures/data_request_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end

set -e CONDA_PREFIX
set -e CONDA_DEFAULT_ENV
set -l vector_count 0
while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue
    test (count $cols) -lt 8; and continue

    set vector_count (math $vector_count + 1)
    set -l op $cols[1]
    set -l cwd (unescape_value "$cols[2]" | string collect -Na)
    set -l format $cols[3]
    set -l is_last (unescape_value "$cols[4]" | string collect -Na)
    set -l is_first (unescape_value "$cols[5]" | string collect -Na)
    set -l prev_bg (unescape_value "$cols[6]" | string collect -Na)
    set -gx VIRTUAL_ENV (unescape_value "$cols[7]" | string collect -Na)
    check "vector $vector_count ($op $cols[2])" "$cols[8]" \
        (__gpy_build_data_payload $op "$cwd" $format "$is_last" "$prev_bg" "$is_first" | string collect)
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "data-request vector tests"
