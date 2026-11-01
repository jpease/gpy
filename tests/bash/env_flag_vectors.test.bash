#!/usr/bin/env bash
# tests/bash/env_flag_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash's __gpy_flag_value maps every row of tests/fixtures/shell_env_flag_vectors.tsv
# to the table's state (also read by the Rust, Zsh and Fish tests). A row is
# on or off when both defaults give that answer, unset when each default is
# returned as is. See the fixture's header for the format.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/shell_env_flag_vectors.tsv"
CASE_COUNT=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

while IFS=$'\t' read -r value_col state_col _rest; do
    [[ -z "$value_col" && -z "$state_col" ]] && continue
    [[ "$value_col" == \#* ]] && continue

    CASE_COUNT=$((CASE_COUNT + 1))
    value="$value_col"
    [[ "$value_col" == '\e' ]] && value=""

    as_on=""
    as_off=""
    __gpy_flag_value "$value" on as_on
    __gpy_flag_value "$value" off as_off
    if [[ "$as_on" == "$as_off" ]]; then
        actual="$as_on"
    else
        actual="unset"
    fi
    check "case $CASE_COUNT ($value_col)" "$state_col" "$actual"
done <"$FIXTURE"

if [[ "$CASE_COUNT" -eq 0 ]]; then
    fail "no test vectors read from $FIXTURE"
fi

test_finish "$CASE_COUNT shell env flag vector tests"
