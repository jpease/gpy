#!/usr/bin/env bash
# tests/bash/json_escape_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #614: __gpy_escape_json must escape control characters (\n, \r, \t) the
# same way Fish's __gpy_json_escape does, not just `\` and `"`. Reads the
# shared vector file tests/fixtures/json_escape_vectors.tsv -- run by all
# three shells -- so escaping can never silently drift between them again.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/json_escape_vectors.tsv"
FAILED=0
CASE_COUNT=0

# Unescape the fixture's `input` column mini-scheme (see the fixture's own
# header comment): backslash first, then control chars.
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

if [[ "$CASE_COUNT" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All $CASE_COUNT JSON-escape vector tests passed ==="
