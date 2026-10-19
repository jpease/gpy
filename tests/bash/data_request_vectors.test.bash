#!/usr/bin/env bash
# tests/bash/data_request_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_build_data_request_into matches Zsh/Fish request bytes for every row of tests/fixtures/data_request_vectors.tsv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

FIXTURE="tests/fixtures/data_request_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# Fixture escapes: `\e` is empty; else backslash first, then \n, \r, \t.
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

unset CONDA_PREFIX CONDA_DEFAULT_ENV
vector_count=0
while IFS=$'\t' read -r op cwd_col format last_col first_col prev_col venv_col expected _rest; do
    [[ -z "$op" || "$op" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    cwd="$(unescape_value "$cwd_col"; printf x)"
    cwd="${cwd%x}"
    prev_bg="$(unescape_value "$prev_col"; printf x)"
    prev_bg="${prev_bg%x}"
    venv="$(unescape_value "$venv_col"; printf x)"
    venv="${venv%x}"
    is_last="$(unescape_value "$last_col")"
    is_first="$(unescape_value "$first_col")"
    label="vector $vector_count ($op $cwd_col)"
    actual="$(out=""; export VIRTUAL_ENV="$venv"; __gpy_build_data_request_into out "$op" "$cwd" "$format" "$is_last" "$prev_bg" "$is_first"; printf '%s' "$out")"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count data-request vectors"
