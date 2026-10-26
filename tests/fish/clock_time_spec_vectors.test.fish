#!/usr/bin/env fish
# tests/fish/clock_time_spec_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_clock_date_format builds the same strftime spec as Bash's and Zsh's
# __gpy_clock_time_spec and the agent, for every row of
# tests/fixtures/clock_time_spec_vectors.tsv (also read by the Bash and Zsh
# tests). See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/segments/clock.fish

# `\e` (the fixture's empty token) -> the empty string.
function column_value --argument-names raw
    test "$raw" = '\e'; or printf '%s' "$raw"
end

set -l fixture "$repo_root/tests/fixtures/clock_time_spec_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end

set -l vector_count 0
while read -d \t -la cols
    test -z "$cols[1]"; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue
    test (count $cols) -lt 4; and continue

    set vector_count (math $vector_count + 1)
    set -g __time_format (column_value $cols[1])
    set -g __clock_show_leading_zero (column_value $cols[2])
    set -g __clock_show_seconds (column_value $cols[3])
    check "vector $vector_count ($cols[1..3])" "$cols[4]" (__gpy_clock_date_format)
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "clock time-spec vector tests"
