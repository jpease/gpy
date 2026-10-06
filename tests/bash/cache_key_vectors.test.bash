#!/usr/bin/env bash
# tests/bash/cache_key_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #705: __gpy_path_to_cache_key must produce the same Unix key as the agent's
# encoder for every row of tests/fixtures/cache_key_vectors.tsv (also read by
# the Rust, Zsh and Fish tests). See the fixture's header for the format.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/cache_key_vectors.tsv"

# Non-ASCII stems count characters only under a UTF-8 locale (#771).
for utf8_locale in C.UTF-8 en_US.UTF-8 ""; do
    [[ -n "$utf8_locale" ]] || { echo "FAIL: no UTF-8 locale available"; exit 1; }
    export LC_ALL="$utf8_locale"
    probe=$'\xc3\xa9'
    (( ${#probe} == 1 )) && break
done
FAILED=0
CASE_COUNT=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

unescape_input() {
    local s="$1"
    s="${s//\\\\/$'\x01'}"
    s="${s//\\n/$'\n'}"
    s="${s//\\r/$'\r'}"
    s="${s//\\t/$'\t'}"
    s="${s//$'\x01'/\\}"
    printf '%s' "$s"
}

while IFS=$'\t' read -r input_col expected_col stem_col _rest; do
    [[ -z "$input_col" && -z "$expected_col" ]] && continue
    [[ "$input_col" == \#* ]] && continue

    CASE_COUNT=$((CASE_COUNT + 1))
    real_input="$(unescape_input "$input_col")"
    actual="$(__gpy_path_to_cache_key "$real_input")"
    if [[ "$actual" == "$expected_col" ]]; then
        echo "PASS: case $CASE_COUNT ($input_col)"
    else
        echo "FAIL: case $CASE_COUNT ($input_col): expected [$expected_col], got [$actual]"
        FAILED=1
    fi
    # #771: the relative cache-file path the reader builds from the key.
    stem="$actual"
    __gpy_chunk_cache_key stem
    if [[ "$stem.git.none.bash" == "$stem_col.git.none.bash" && -n "$stem_col" ]]; then
        echo "PASS: case $CASE_COUNT stem"
    else
        echo "FAIL: case $CASE_COUNT stem: expected [$stem_col], got [$stem]"
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
echo "=== All $CASE_COUNT cache-key vector tests passed ==="
