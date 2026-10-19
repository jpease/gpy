#!/usr/bin/env zsh
# tests/zsh/json_flags_tail.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_json_flags_tail_into is the one place the is_last/is_first/prev_bg JSON
# tail is built for every IPC request builder in zsh/core/ipc.zsh. This
# feeds it every row of tests/fixtures/json_flags_tail_vectors.tsv (also read
# by the Bash and Fish tests), so the three shells' tails stay byte-identical.
# See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/json_flags_tail_vectors.tsv"
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

vector_count=0
while IFS=$'\t' read -r last_col first_col prev_col expected _rest; do
    [[ -z "$last_col" || "$last_col" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    is_last=$(unescape_value "$last_col")
    is_first=$(unescape_value "$first_col")
    prev_bg=$(unescape_value "$prev_col"; printf x)
    prev_bg=${prev_bg%x}
    [[ "$expected" == '\e' ]] && expected=""
    label="vector $vector_count ($last_col $first_col $prev_col)"
    __gpy_json_flags_tail_into actual "$is_last" "$is_first" "$prev_bg"
    if [[ "$actual" == "$expected" ]]; then
        pass "$label"
    else
        fail "$label: expected [$expected], got [$actual]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count json_flags_tail vectors"
