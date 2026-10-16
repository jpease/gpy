#!/usr/bin/env zsh
# tests/zsh/status_icon_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_status_icon picks the same icon as Bash's copy for every row of
# tests/fixtures/status_icon_vectors.tsv (also read by the Bash test). See the
# fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/renderer.zsh

FIXTURE="$ROOT/tests/fixtures/status_icon_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# `apply NAME COLUMN`: \u unsets NAME, \e sets it empty, anything else sets it.
apply() {
    case $2 in
        '\u') unset "$1" ;;
        '\e') typeset -g "$1"="" ;;
        *) typeset -g "$1"="$2" ;;
    esac
}

vector_count=0
while IFS=$'\t' read -r icons_col ok_col fail_col ok_expected fail_expected _rest; do
    [[ -z "$icons_col" || "$icons_col" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    [[ "$ok_expected" == '\e' ]] && ok_expected=""
    [[ "$fail_expected" == '\e' ]] && fail_expected=""
    apply __prompt_icons "$icons_col"
    apply __icon_status_ok "$ok_col"
    apply __icon_status_fail "$fail_col"
    ok="" bad=""
    __gpy_status_icon ok ok
    __gpy_status_icon fail bad
    label="vector $vector_count ($icons_col $ok_col $fail_col)"
    if [[ "$ok" == "$ok_expected" && "$bad" == "$fail_expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$ok_expected|$fail_expected], got [$ok|$bad]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count status-icon vectors"
