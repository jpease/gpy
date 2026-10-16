#!/usr/bin/env zsh
# tests/zsh/uint_timeout_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_ms_to_secs validates GPY_IPC_TIMEOUT_MS by the same rule as Fish's
# __gpy_uint_or_default and Bash's __gpy_ms_to_secs, and reads it as decimal,
# for every row of tests/fixtures/uint_timeout_vectors.tsv (also read by the
# Bash and Fish tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/uint_timeout_vectors.tsv"
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
    # A subshell, so an arithmetic error cannot abort the whole test.
    actual=$(__gpy_ms_to_secs "$value" secs 2>&1 && printf '%s' "$secs")
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count uint/timeout vectors"
