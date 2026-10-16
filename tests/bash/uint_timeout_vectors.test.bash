#!/usr/bin/env bash
# tests/bash/uint_timeout_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_ms_to_secs matches Fish/Zsh on every row of tests/fixtures/uint_timeout_vectors.tsv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

FIXTURE="tests/fixtures/uint_timeout_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

vector_count=0
while IFS=$'\t' read -r value _validated expected _rest; do
    [[ -z "$value" || "$value" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    [[ "$value" == '\e' ]] && value=""
    label="vector $vector_count [$value]"
    secs=""
    # A subshell, so an arithmetic error cannot abort the whole test.
    actual="$(__gpy_ms_to_secs "$value" secs 2>&1 && printf '%s' "$secs")"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count uint/timeout vectors"
