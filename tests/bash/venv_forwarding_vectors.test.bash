#!/usr/bin/env bash
# tests/bash/venv_forwarding_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #729: Bash's __gpy_forwarded_venv picks the same `virtual_env` value as every
# row of tests/fixtures/venv_forwarding_vectors.tsv (also read by the Rust and
# Zsh/Fish tests). See the fixture's header for the format.

# Each lookup runs in its own subshell on purpose, so the env never leaks.
# shellcheck disable=SC2030,SC2031
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/venv_forwarding_vectors.tsv"
FAILED=0
CASE_COUNT=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

# `\e` is the empty string (an empty column would be collapsed by `read`).
col() { [[ "$1" == '\e' ]] && printf '' || printf '%s' "$1"; }

while IFS=$'\t' read -r venv_col prefix_col env_col expected_col _rest; do
    [[ -z "$venv_col" && -z "$prefix_col" ]] && continue
    [[ "$venv_col" == \#* ]] && continue

    CASE_COUNT=$((CASE_COUNT + 1))
    venv="$(col "$venv_col")"
    prefix="$(col "$prefix_col")"
    conda_env="$(col "$env_col")"
    expected="$(col "$expected_col")"

    # Empty and unset must behave alike: run once with the variable exported
    # empty and once with it unset.
    actual="$(export VIRTUAL_ENV="$venv" CONDA_PREFIX="$prefix" CONDA_DEFAULT_ENV="$conda_env"; __gpy_forwarded_venv)"
    unset_actual="$(unset VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV; [[ -n "$venv" ]] && export VIRTUAL_ENV="$venv"; [[ -n "$prefix" ]] && export CONDA_PREFIX="$prefix"; [[ -n "$conda_env" ]] && export CONDA_DEFAULT_ENV="$conda_env"; __gpy_forwarded_venv)"
    for got in "$actual" "$unset_actual"; do
        if [[ "$got" == "$expected" ]]; then
            echo "PASS: case $CASE_COUNT ($venv_col|$prefix_col|$env_col)"
        else
            echo "FAIL: case $CASE_COUNT ($venv_col|$prefix_col|$env_col): expected [$expected], got [$got]"
            FAILED=1
        fi
    done
done < "$FIXTURE"

if [[ "$CASE_COUNT" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All $CASE_COUNT venv forwarding vector tests passed ==="
