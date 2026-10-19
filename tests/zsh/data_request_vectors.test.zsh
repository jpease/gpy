#!/usr/bin/env zsh
# tests/zsh/data_request_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_build_data_request_into builds the same git/lang request bytes as
# Bash's __gpy_build_data_request_into and Fish's __gpy_build_data_payload for
# every row of tests/fixtures/data_request_vectors.tsv (also read by the Bash
# and Fish tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/data_request_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# The fixture's mini escape scheme: `\e` is the empty string; otherwise
# backslash first, then \n, \r, \t.
unescape_value() {
    local s=$1
    [[ $s == '\e' ]] && return
    s=${s//\\\\/$'\x01'}
    s=${s//\\n/$'\n'}
    s=${s//\\r/$'\r'}
    s=${s//\\t/$'\t'}
    s=${s//$'\x01'/\\}
    printf '%s' "$s"
}

unset CONDA_PREFIX CONDA_DEFAULT_ENV
vector_count=0
while IFS=$'\t' read -r op cwd_col format last_col first_col prev_col venv_col expected _rest; do
    [[ -z "$op" || "$op" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    cwd=$(unescape_value "$cwd_col"; printf x)
    cwd=${cwd%x}
    prev_bg=$(unescape_value "$prev_col"; printf x)
    prev_bg=${prev_bg%x}
    venv=$(unescape_value "$venv_col"; printf x)
    venv=${venv%x}
    is_last=$(unescape_value "$last_col")
    is_first=$(unescape_value "$first_col")
    label="vector $vector_count ($op $cwd_col)"
    actual=$(export VIRTUAL_ENV="$venv"; __gpy_build_data_request_into out "$op" "$cwd" "$format" "$is_last" "$prev_bg" "$is_first"; printf '%s' "$out")
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count data-request vectors"
