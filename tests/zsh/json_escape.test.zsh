#!/usr/bin/env zsh
# tests/zsh/json_escape.test.zsh
#
# Regression test for #302: __gpy_escape_json used `s=${s//\/\\}`, which in
# zsh's pattern-substitution syntax matches the literal two-character
# sequence `/\` and deletes it — it never escapes a lone backslash at all.
# A cwd or branch name containing `\` (e.g. a Windows-style path) then
# produced malformed JSON, which the agent rejects, silently dropping the
# segment. The fix is `s=${s//\\/\\\\}`, matching bash/core/ipc.bash.

ROOT=${0:a:h:h:h}
cd "$ROOT"

source zsh/core/ipc.zsh

test_result=0

if ! command -v jq &>/dev/null; then
    echo "SKIP: jq not installed, cannot validate JSON round-trip"
else

run_case() {
    local label=$1
    local input=$2

    local escaped
    escaped=$(__gpy_escape_json "$input")

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

fi

# #614: __gpy_escape_json must also escape control characters (\n, \r, \t)
# the same way Fish's __gpy_json_escape does. Read the shared vector file
# (tests/fixtures/json_escape_vectors.tsv) so escaping can never silently
# drift between bash/zsh/fish again -- see the fixture's own header comment
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
    actual=$(__gpy_escape_json "$real_input")
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
