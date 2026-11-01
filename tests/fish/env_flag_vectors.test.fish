#!/usr/bin/env fish
# tests/fish/env_flag_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fish's __gpy_flag_value maps every row of tests/fixtures/shell_env_flag_vectors.tsv
# to the table's state (also read by the Rust, Bash and Zsh tests). A row is
# on or off when both defaults give that answer, unset when each default is
# returned as is. See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/flags.fish

set -l fixture "$repo_root/tests/fixtures/shell_env_flag_vectors.tsv"
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
    set -l value_col $cols[1]
    set -l value "$value_col"
    test "$value_col" = '\e'; and set value ""

    set -l as_on (__gpy_flag_value "$value" on)
    set -l as_off (__gpy_flag_value "$value" off)
    set -l actual unset
    test "$as_on" = "$as_off"; and set actual $as_on
    check "vector $vector_count ($value_col)" "$cols[2]" "$actual"
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "shell env flag vector tests"
