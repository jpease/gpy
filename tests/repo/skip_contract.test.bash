#!/usr/bin/env bash
# tests/repo/skip_contract.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A skipped test is a failure under CI or GPY_GATE_STRICT, and passes locally
# otherwise. Covers `test_skip` in Bash, Zsh (tests/lib/shell_e2e.sh) and Fish
# (tests/lib/test_helpers.fish). Every case scrubs both variables first.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command zsh

# check_skip LABEL EXPECTED_RC COMMAND [VAR=VALUE...] -- run COMMAND with CI and
# GPY_GATE_STRICT scrubbed (plus the given variables) and verify the exit code
# and the `SKIP:` line.
check_skip() {
    local label="$1" expected="$2" cmd_shell="$3" snippet="$4"
    shift 4
    local out rc
    out="$(env -u CI -u GPY_GATE_STRICT "$@" "$cmd_shell" -c "$snippet" 2>&1)"
    rc=$?
    if [[ "$rc" -eq "$expected" && "$out" == *"SKIP: demo"* ]]; then
        pass "$label: exit $expected with SKIP line"
    else
        fail "$label: expected exit $expected with 'SKIP: demo', got $rc: $out"
    fi
}

bash_snippet=". '$ROOT/tests/lib/shell_e2e.sh'; test_skip demo"
fish_snippet="source '$ROOT/tests/lib/test_helpers.fish'; test_skip demo"

for variant in "unset:0" "GPY_GATE_STRICT=1:1" "CI=1:1"; do
    var="${variant%:*}"
    expected="${variant##*:}"
    extra=()
    [[ "$var" == unset ]] || extra=("$var")
    check_skip "bash test_skip ($var)" "$expected" bash "$bash_snippet" "${extra[@]}"
    check_skip "zsh test_skip ($var)" "$expected" zsh "$bash_snippet" "${extra[@]}"
    check_skip "fish test_skip ($var)" "$expected" fish "$fish_snippet" "${extra[@]}"
done

test_finish
