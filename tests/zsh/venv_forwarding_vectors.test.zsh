#!/usr/bin/env zsh
# tests/zsh/venv_forwarding_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #729: Zsh's __gpy_forwarded_venv picks the same `virtual_env` value as every
# row of tests/fixtures/venv_forwarding_vectors.tsv (also read by the Rust and
# Bash/Fish tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/venv_forwarding_vectors.tsv"
test_result=0
vector_count=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

# `\e` is the empty string (an empty column would be collapsed by `read`).
col() { [[ "$1" == '\e' ]] && printf '' || printf '%s' "$1"; }

while IFS=$'\t' read -r venv_col prefix_col env_col expected_col _rest; do
    [[ -z "$venv_col" && -z "$prefix_col" ]] && continue
    [[ "$venv_col" == \#* ]] && continue

    vector_count=$((vector_count + 1))
    venv=$(col "$venv_col")
    prefix=$(col "$prefix_col")
    conda_env=$(col "$env_col")
    expected=$(col "$expected_col")

    # Empty and unset must behave alike.
    actual=$(export VIRTUAL_ENV="$venv" CONDA_PREFIX="$prefix" CONDA_DEFAULT_ENV="$conda_env"; __gpy_forwarded_venv)
    unset_actual=$(unset VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV; [[ -n "$venv" ]] && export VIRTUAL_ENV="$venv"; [[ -n "$prefix" ]] && export CONDA_PREFIX="$prefix"; [[ -n "$conda_env" ]] && export CONDA_DEFAULT_ENV="$conda_env"; __gpy_forwarded_venv)
    for got in "$actual" "$unset_actual"; do
        if [[ "$got" == "$expected" ]]; then
            print -r -- "PASS: vector $vector_count ($venv_col|$prefix_col|$env_col)"
        else
            print -r -- "FAIL: vector $vector_count ($venv_col|$prefix_col|$env_col): expected [$expected], got [$got]"
            test_result=1
        fi
    done
done < "$FIXTURE"

if [[ "$vector_count" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ "$test_result" -eq 0 ]]; then
    echo "PASS: all $vector_count venv forwarding vector tests passed"
else
    echo "FAIL"
fi
exit $test_result
