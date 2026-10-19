#!/usr/bin/env bash
# tests/bash/json_flags_tail.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_json_flags_tail_into must match tests/fixtures/json_flags_tail_vectors.tsv byte-for-byte.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

FIXTURE="tests/fixtures/json_flags_tail_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# Fixture escapes: `\e` is empty; otherwise backslash first, then \n, \r, \t.
unescape_value() {
    local s="$1"
    [[ "$s" == '\e' ]] && return
    s="${s//\\\\/$'\x01'}"
    s="${s//\\n/$'\n'}"
    s="${s//\\r/$'\r'}"
    s="${s//\\t/$'\t'}"
    s="${s//$'\x01'/\\}"
    printf '%s' "$s"
}

vector_count=0
while IFS=$'\t' read -r last_col first_col prev_col expected _rest; do
    [[ -z "$last_col" || "$last_col" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    is_last="$(unescape_value "$last_col")"
    is_first="$(unescape_value "$first_col")"
    prev_bg="$(unescape_value "$prev_col"; printf x)"
    prev_bg="${prev_bg%x}"
    [[ "$expected" == '\e' ]] && expected=""
    label="vector $vector_count ($last_col $first_col $prev_col)"
    actual=""
    __gpy_json_flags_tail_into actual "$is_last" "$is_first" "$prev_bg"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count json_flags_tail vectors"
