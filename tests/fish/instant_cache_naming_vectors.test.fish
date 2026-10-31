#!/usr/bin/env fish
# tests/fish/instant_cache_naming_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fish names instant-cache files (prev_bg token, variant suffix) and reads the
# cache-read status code exactly as Bash, Zsh and the agent do, for every row
# of tests/fixtures/instant_cache_naming_vectors.tsv (also read by the Bash and
# Zsh tests). See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish
source fish/core/util.fish

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

function yes_no
    if $argv
        echo yes
    else
        echo no
    end
end

set -l fixture "$repo_root/tests/fixtures/instant_cache_naming_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end

set -l vector_count 0
while read -d \t -la cols
    test -z "$cols[1]"; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue

    set vector_count (math $vector_count + 1)
    set -l label "vector $vector_count ($cols[1..4])"
    switch $cols[1]
        case token
            set -l value (unescape_value "$cols[2]" | string collect -Na)
            check "$label" "$cols[3]" (__gpy_prev_bg_token "$value" | string collect)
        case suffix
            set -l is_last (unescape_value "$cols[3]" | string collect -Na)
            set -l is_first (unescape_value "$cols[4]" | string collect -Na)
            check "$label" "$cols[5]" (__gpy_cache_variant_suffix $cols[2] "$is_last" "$is_first" | string collect)
        case bgfile
            check "$label" "$cols[3]" (__gpy_git_bg_file_tail "$cols[2]" | string collect)
        case status
            check "$label stale" "$cols[3]" (yes_no __gpy_cache_status_stale $cols[2])
            check "$label variant" "$cols[4]" (yes_no __gpy_cache_status_variant $cols[2])
        case '*'
            fail "$label: unknown row kind [$cols[1]]"
    end
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "instant-cache naming vector tests"
