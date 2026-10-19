#!/usr/bin/env bash
# tests/bash/json_escape_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_escape_json must escape control chars like Fish does, per the shared
# tests/fixtures/json_escape_vectors.tsv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/json_escape_vectors.tsv"
FAILED=0
CASE_COUNT=0

# Fixture input mini-scheme: backslash first, then control chars.
unescape_input() {
    local s="$1"
    s="${s//\\\\/$'\x01'}"   # temporarily protect literal backslash pairs
    s="${s//\\n/$'\n'}"
    s="${s//\\r/$'\r'}"
    s="${s//\\t/$'\t'}"
    s="${s//$'\x01'/\\}"
    printf '%s' "$s"
}

while IFS=$'\t' read -r input_col expected_col; do
    [[ -z "$input_col" && -z "$expected_col" ]] && continue
    [[ "$input_col" == \#* ]] && continue

    CASE_COUNT=$((CASE_COUNT + 1))
    real_input="$(unescape_input "$input_col")"
    actual="$(__gpy_escape_json "$real_input")"
    if [[ "$actual" == "$expected_col" ]]; then
        echo "PASS: case $CASE_COUNT ($input_col)"
    else
        echo "FAIL: case $CASE_COUNT ($input_col): expected [$expected_col], got [$actual]"
        FAILED=1
    fi
done < "$FIXTURE"

# __gpy_escape_json_into must not fork: the DEBUG trap (inherited via
# functrace) logs any command run in a subshell during the call.
forks_file="$(mktemp "${TMPDIR:-/tmp}/gpy-ej-forks.XXXXXX")"
no_fork_out=""
set -T
trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "$BASH_COMMAND" >>"$forks_file"' DEBUG
__gpy_escape_json_into no_fork_out $'a\\b"c\nd\re\tf'
trap - DEBUG
set +T
CASE_COUNT=$((CASE_COUNT + 1))
if [[ ! -s "$forks_file" && "$no_fork_out" == 'a\\b\"c\nd\re\tf' ]]; then
    echo "PASS: case $CASE_COUNT (__gpy_escape_json_into forks no subshell)"
else
    echo "FAIL: case $CASE_COUNT (__gpy_escape_json_into): forks [$(<"$forks_file")], got [$no_fork_out]"
    FAILED=1
fi
rm -f "$forks_file"

if [[ "$CASE_COUNT" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All $CASE_COUNT JSON-escape vector tests passed ==="
