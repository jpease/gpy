#!/usr/bin/env bash
# tests/bash/git_root_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_find_git_root and __gpy_instant_cache_key_path find the same
# directory as Zsh's and Fish's for every row of
# tests/fixtures/git_root_vectors.tsv (also read by the Zsh and Fish tests).
# See the fixture's header for the format.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

FIXTURE="tests/fixtures/git_root_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

S="$(mktemp -d "${TMPDIR:-/tmp}/gpy-git-root.XXXXXX")" || exit 1
trap 'rm -rf "$S"' EXIT
S="$(realpath "$S")"

# S-relative path -> absolute; `.` is S itself, \e stays "none" (empty).
abs() {
    case "$1" in
        '\e') ;;
        .) printf '%s' "$S" ;;
        *) printf '%s' "$S/$1" ;;
    esac
}

vector_count=0
while IFS=$'\t' read -r kind c2 c3 c4 _rest; do
    [[ -z "$kind" || "$kind" == \#* ]] && continue
    case "$kind" in
        mkdir) mkdir -p "$S/$c2" ;;
        gitdir) mkdir -p "$S/$c2/.git" ;;
        gitfile) mkdir -p "$S/$c2" && printf 'gitdir: %s\n' "$S/.worktree-admin" > "$S/$c2/.git" ;;
        symlink) ln -s "$S/$c3" "$S/$c2" ;;
        root)
            vector_count=$((vector_count + 1))
            expected="$(abs "$c3")"
            actual=""
            __gpy_find_git_root "$S/$c2" actual
            status=$?
            [[ -n "$expected" || $status -ne 0 ]] || fail "root $c2: found nothing but returned 0"
            check "root vector $vector_count ($c2)" "$expected" "$actual"
            ;;
        key)
            vector_count=$((vector_count + 1))
            expected="$(abs "$c4")"
            actual=""
            __gpy_instant_cache_key_path "$c2" "$S/$c3" actual
            status=$?
            [[ -n "$expected" || $status -ne 0 ]] || fail "key $c2 $c3: found nothing but returned 0"
            [[ -z "$expected" || $status -eq 0 ]] || fail "key $c2 $c3: found a key but returned $status"
            check "key vector $vector_count ($c2 $c3)" "$expected" "$actual"
            ;;
        *) fail "unknown row kind [$kind]" ;;
    esac
done < "$FIXTURE"

# The $PWD memo resolves a symlinked working directory like realpath does.
cd "$S/link" || exit 1; __gpy_resolve_pwd
actual=""; __gpy_find_git_root "$PWD" actual
check "memoized symlinked \$PWD git root" "$S/repo" "$actual"
cd "$S/link_plain" || exit 1; __gpy_resolve_pwd
actual=""; __gpy_instant_cache_key_path lang "$PWD" actual
check "memoized symlinked \$PWD lang key" "$S/plain/sub" "$actual"
cd "$ROOT" || exit 1

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count git-root vectors"
