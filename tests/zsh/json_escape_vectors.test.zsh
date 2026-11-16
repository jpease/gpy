#!/usr/bin/env zsh
# tests/zsh/json_escape_vectors.test.zsh
#
# __gpy_escape_json_into must escape a lone backslash. In zsh's
# pattern-substitution syntax `s=${s//\/\\}` matches the literal
# two-character sequence `/\` and deletes it, never escaping a lone backslash;
# a cwd or branch name containing `\` (e.g. a Windows-style path) would then
# produce malformed JSON, which the agent rejects, silently dropping the
# segment. The correct form is `s=${s//\\/\\\\}`, matching bash/core/ipc.bash.

ROOT=${0:a:h:h:h}
cd "$ROOT"
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

source zsh/core/ipc.zsh

test_result=0

if ! command -v jq &>/dev/null; then
    test_skip_part "jq not installed, cannot validate JSON round-trip"
else

run_case() {
    local label=$1
    local input=$2

    local escaped
    __gpy_escape_json_into escaped "$input"

    local json_string
    json_string="{\"test\":\"${escaped}\"}"

    if ! print -r -- "$json_string" | jq . >/dev/null 2>&1; then
        echo "FAIL: $label produced invalid JSON: $json_string"
        test_result=1
        return
    fi

    local extracted
    extracted=$(print -r -- "$json_string" | jq -r '.test')

    if [[ "$extracted" != "$input" ]]; then
        echo "FAIL: $label round-trip mismatch: got '$extracted' expected '$input'"
        test_result=1
        return
    fi

    echo "PASS: $label round-trips correctly"
}

run_case "single backslash" 'a\b'
run_case "double backslash" 'a\\b'
run_case "trailing backslash" 'C:\Users\foo\'
run_case "windows-style path" 'C:\Users\foo\bar'
run_case "double quote" 'a"b'
run_case "backslash and quote" 'a\"b'
run_case "slash and backslash" 'x/\y'
# Every other C0 control and DEL goes out as \u00XX, so a cwd holding one
# (ESC, BEL, a Latin-1 name's control byte) still makes a valid request.
run_case "escape (ESC)" $'/tmp/a\x1bb'
run_case "bell, backspace, form feed, vertical tab" $'\a\b\f\v'
run_case "SOH and US" $'\x01x\x1f'
run_case "DEL" $'a\x7fb'
run_case "controls beside newline, tab and quote" $'a\x1b\n\t"\x02'

fi

# The exact form, independent of jq.
for pair in $'\x1b=\\u001b' $'\x01=\\u0001' $'\x1f=\\u001f' $'\x7f=\\u007f'; do
    __gpy_escape_json_into escaped_c0 "${pair%%=*}"
    if [[ "$escaped_c0" == "${pair#*=}" ]]; then
        print -r -- "PASS: control byte escapes to ${pair#*=}"
    else
        print -r -- "FAIL: control byte escaped to [$escaped_c0], want [${pair#*=}]"
        test_result=1
    fi
done
unset pair escaped_c0

# __gpy_escape_json_into must also escape control characters (\n, \r, \t)
# the same way Fish's __gpy_json_escape does. Read the shared vector file
# (tests/fixtures/json_escape_vectors.tsv) so escaping cannot silently
# drift between bash/zsh/fish -- see the fixture's own header comment
# for the input-column mini escape scheme.
FIXTURE="$ROOT/tests/fixtures/json_escape_vectors.tsv"
vector_count=0

unescape_input() {
    local s=$1
    s=${s//\\\\/$'\x01'}
    s=${s//\\n/$'\n'}
    s=${s//\\r/$'\r'}
    s=${s//\\t/$'\t'}
    s=${s//$'\x01'/\\}
    printf '%s' "$s"
}

while IFS=$'\t' read -r input_col expected_col; do
    [[ -z "$input_col" && -z "$expected_col" ]] && continue
    [[ "$input_col" == \#* ]] && continue

    vector_count=$((vector_count + 1))
    real_input=$(unescape_input "$input_col")
    __gpy_escape_json_into actual "$real_input"
    if [[ "$actual" == "$expected_col" ]]; then
        print -r -- "PASS: vector $vector_count ($input_col) round-trips correctly"
    else
        print -r -- "FAIL: vector $vector_count ($input_col): expected [$expected_col], got [$actual]"
        test_result=1
    fi
done < "$FIXTURE"

if [[ "$vector_count" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    test_result=1
fi

if [[ "$test_result" -eq 0 ]]; then
    echo "PASS"
else
    echo "FAIL"
fi

exit $test_result
