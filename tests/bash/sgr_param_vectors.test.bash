#!/usr/bin/env bash
# tests/bash/sgr_param_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_sgr_param matches Zsh's copy for every row of tests/fixtures/sgr_param_vectors.tsv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/renderer.bash
source bash/core/renderer.bash

FIXTURE="tests/fixtures/sgr_param_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

vector_count=0
while IFS=$'\t' read -r color fg_expected bg_expected _rest; do
    [[ -z "$color" || "$color" == '#' || "$color" == '# '* ]] && continue
    vector_count=$((vector_count + 1))
    [[ "$color" == '\e' ]] && color=""
    fg="" bg=""
    __gpy_sgr_param fg "$color" fg
    __gpy_sgr_param bg "$color" bg
    if [[ "$fg" == "$fg_expected" && "$bg" == "$bg_expected" ]]; then
        pass "vector $vector_count [$color]"
    else
        fail "vector $vector_count [$color]: expected [$fg_expected|$bg_expected], got [$fg|$bg]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count SGR parameter vectors"
