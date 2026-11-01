#!/usr/bin/env zsh
# tests/zsh/env_flag_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh's __gpy_flag_value maps every row of tests/fixtures/shell_env_flag_vectors.tsv
# to the table's state (also read by the Rust, Bash and Fish tests). A row is
# on or off when both defaults give that answer, unset when each default is
# returned as is. See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/shell_env_flag_vectors.tsv"
vector_count=0

if [[ ! -r "$FIXTURE" ]]; then
    fail "cannot read $FIXTURE"
    test_finish
fi

while IFS=$'\t' read -r value_col state_col _rest; do
    [[ -z "$value_col" && -z "$state_col" ]] && continue
    [[ "$value_col" == \#* ]] && continue

    vector_count=$((vector_count + 1))
    value=$value_col
    [[ "$value_col" == '\e' ]] && value=""

    as_on=""
    as_off=""
    __gpy_flag_value "$value" on as_on
    __gpy_flag_value "$value" off as_off
    if [[ "$as_on" == "$as_off" ]]; then
        actual=$as_on
    else
        actual=unset
    fi
    check "vector $vector_count ($value_col)" "$state_col" "$actual"
done <"$FIXTURE"

if [[ "$vector_count" -eq 0 ]]; then
    fail "no test vectors read from $FIXTURE"
fi

test_finish "$vector_count shell env flag vector tests"
