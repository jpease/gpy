#!/usr/bin/env zsh
# tests/zsh/clock_time_spec_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_clock_time_spec builds the same strftime spec as Bash's copy, Fish's
# __gpy_clock_date_format and the agent, for every row of
# tests/fixtures/clock_time_spec_vectors.tsv (also read by the Bash and Fish
# tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/segments/clock.zsh

FIXTURE="$ROOT/tests/fixtures/clock_time_spec_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

vector_count=0
while IFS=$'\t' read -r format_col zero_col seconds_col expected _rest; do
    [[ -z "$format_col" || "$format_col" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    __time_format=$format_col
    __clock_show_leading_zero=$zero_col
    __clock_show_seconds=$seconds_col
    [[ "$__time_format" == '\e' ]] && __time_format=""
    [[ "$__clock_show_leading_zero" == '\e' ]] && __clock_show_leading_zero=""
    [[ "$__clock_show_seconds" == '\e' ]] && __clock_show_seconds=""
    label="vector $vector_count ($format_col $zero_col $seconds_col)"

    # Both calling conventions: printed, and assigned into OUT_VAR.
    actual=$(__gpy_clock_time_spec)
    __gpy_clock_time_spec spec
    if [[ "$actual" == "$expected" && "$spec" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got printed [$actual] / assigned [$spec]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count clock time-spec vectors"
