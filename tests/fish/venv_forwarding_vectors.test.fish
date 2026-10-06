#!/usr/bin/env fish
# tests/fish/venv_forwarding_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #729: Fish's __gpy_forwarded_venv picks the same `virtual_env` value as every
# row of tests/fixtures/venv_forwarding_vectors.tsv (also read by the Rust and
# Bash/Zsh tests). See the fixture's header for the format.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish
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

# `\e` is the empty string (an empty column would be collapsed by `read`).
function col --argument-names raw
    test "$raw" = '\e'; and return
    printf '%s' "$raw"
end

set -l fixture "$repo_root/tests/fixtures/venv_forwarding_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end
set -l vector_count 0

while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    set -l first_char (string sub -l 1 -- "$cols[1]")
    test "$first_char" = '#'; and continue
    test (count $cols) -lt 4; and continue

    set vector_count (math $vector_count + 1)
    set -l expected (col "$cols[4]" | string collect -a)

    # Empty and unset must behave alike.
    set -gx VIRTUAL_ENV (col "$cols[1]" | string collect -a)
    set -gx CONDA_PREFIX (col "$cols[2]" | string collect -a)
    set -gx CONDA_DEFAULT_ENV (col "$cols[3]" | string collect -a)
    check "vector $vector_count ($cols[1]|$cols[2]|$cols[3]) empty" "$expected" (__gpy_forwarded_venv | string collect -a)

    set -e VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV
    test "$cols[1]" != '\e'; and set -gx VIRTUAL_ENV $cols[1]
    test "$cols[2]" != '\e'; and set -gx CONDA_PREFIX $cols[2]
    test "$cols[3]" != '\e'; and set -gx CONDA_DEFAULT_ENV $cols[3]
    check "vector $vector_count ($cols[1]|$cols[2]|$cols[3]) unset" "$expected" (__gpy_forwarded_venv | string collect -a)
    set -e VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count venv forwarding vector tests passed"
