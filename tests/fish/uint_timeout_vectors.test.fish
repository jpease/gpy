#!/usr/bin/env fish
# tests/fish/uint_timeout_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_uint_or_default validates a numeric GPY_* setting by the same rule
# Bash's and Zsh's __gpy_ms_to_secs apply to GPY_IPC_TIMEOUT_MS, for every row
# of tests/fixtures/uint_timeout_vectors.tsv (also read by the Bash and Zsh
# tests). See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish

set -l fixture "$repo_root/tests/fixtures/uint_timeout_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end

set -l vector_count 0
while read -d \t -la cols
    test -z "$cols[1]"; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue
    test (count $cols) -lt 3; and continue

    set vector_count (math $vector_count + 1)
    set -l value "$cols[1]"
    test "$value" = '\e'; and set value ''
    check "vector $vector_count [$value]" "$cols[2]" (__gpy_uint_or_default "$value" 150)
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "uint/timeout vector tests"
