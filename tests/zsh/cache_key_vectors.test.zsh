#!/usr/bin/env zsh
# tests/zsh/cache_key_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #705: __gpy_path_to_cache_key must produce the same Unix key as the agent's
# encoder for every row of tests/fixtures/cache_key_vectors.tsv (also read by
# the Rust, Bash and Fish tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/cache_key_vectors.tsv"

# Non-ASCII stems count characters only under a UTF-8 locale (#771).
for utf8_locale in C.UTF-8 en_US.UTF-8 ""; do
    [[ -n "$utf8_locale" ]] || { echo "FAIL: no UTF-8 locale available"; exit 1; }
    export LC_ALL=$utf8_locale
    probe=$'\xc3\xa9'
    (( ${#probe} == 1 )) && break
done
test_result=0
vector_count=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

unescape_input() {
    local s=$1
    s=${s//\\\\/$'\x01'}
    s=${s//\\n/$'\n'}
    s=${s//\\r/$'\r'}
    s=${s//\\t/$'\t'}
    s=${s//$'\x01'/\\}
    printf '%s' "$s"
}

while IFS=$'\t' read -r input_col expected_col stem_col _rest; do
    [[ -z "$input_col" && -z "$expected_col" ]] && continue
    [[ "$input_col" == \#* ]] && continue

    vector_count=$((vector_count + 1))
    real_input=$(unescape_input "$input_col")
    actual=$(__gpy_path_to_cache_key "$real_input")
    if [[ "$actual" == "$expected_col" ]]; then
        print -r -- "PASS: vector $vector_count ($input_col)"
    else
        print -r -- "FAIL: vector $vector_count ($input_col): expected [$expected_col], got [$actual]"
        test_result=1
    fi
    # #771: the relative cache-file path the reader builds from the key.
    stem=$actual
    __gpy_chunk_cache_key stem
    if [[ -n "$stem_col" && "$stem.git.none.zsh" == "$stem_col.git.none.zsh" ]]; then
        print -r -- "PASS: vector $vector_count stem"
    else
        print -r -- "FAIL: vector $vector_count stem: expected [$stem_col], got [$stem]"
        test_result=1
    fi
done < "$FIXTURE"

if [[ "$vector_count" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ "$test_result" -eq 0 ]]; then
    echo "PASS: all $vector_count cache-key vector tests passed"
else
    echo "FAIL"
fi
exit $test_result
