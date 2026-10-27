#!/usr/bin/env bash
# tests/bash/script_runner_source.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shellcheck disable=SC2015  # pass never fails, so A && pass || fail is if/else
# shellcheck disable=SC2016  # the planted text must stay literal
#
# Source rules for the script (G) segment runner, bash/core/g_runner.bash:
#   (a) zsh/core and fish/core hold byte-identical copies (they run it in a
#       Bash child; tracked symlinks are not allowed)
#   (b) no comment holds an apostrophe: Bash 3.2 mis-parses one inside a
#       process substitution
#   (c) no `command` and no bare `sleep`: in a Bash 3.2 timer job
#       `command sleep` never returned, and a PATH search stats the working
#       directory first, which blocks on a hung mount
# Each detector is also shown firing on a planted violation.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

RUNNER=bash/core/g_runner.bash
WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-runner-src.XXXXXX")"
trap 'rm -rf "$WORKDIR"' EXIT

comment_apostrophes() {
    grep -nE "^[[:space:]]*#.*'|[[:space:]]#[^\"]*'[^\"]*$" "$1"
}
path_searched_sleep() {
    grep -vE '^[[:space:]]*#' "$1" | grep -E "(^|[;&|({[:space:]])(command|sleep)[[:space:]]"
}

echo "=== (a) copies ==="
for copy in zsh/core/g_runner.bash fish/core/g_runner.bash; do
    if [[ ! -f "$copy" ]]; then
        fail "$copy is missing"
    elif cmp -s "$RUNNER" "$copy"; then
        pass "$copy is identical to $RUNNER"
    else
        fail "$copy differs from $RUNNER (cp $RUNNER $copy)"
    fi
done

echo "=== (b) no apostrophe in a comment ==="
hits="$(comment_apostrophes "$RUNNER")"
check "no comment line in $RUNNER holds an apostrophe" "" "$hits"
printf '    # it'\''s planted\n' >"$WORKDIR/planted"
[[ -n "$(comment_apostrophes "$WORKDIR/planted")" ]] && pass "the detector finds a planted apostrophe" ||
    fail "the apostrophe detector missed a planted comment"

echo "=== (c) no PATH-searched sleep or command ==="
hits="$(path_searched_sleep "$RUNNER")"
check "$RUNNER runs sleep only through \$__gpy_g_sleep" "" "$hits"
printf '    command sleep "$t"\n' >"$WORKDIR/planted"
[[ -n "$(path_searched_sleep "$WORKDIR/planted")" ]] && pass "the detector finds a planted command sleep" ||
    fail "the sleep detector missed a planted line"

test_finish "script runner source rules"
