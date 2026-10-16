#!/usr/bin/env zsh
# tests/zsh/git_root_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_find_git_root and __gpy_instant_cache_key_path find the same
# directory as Bash's and Fish's for every row of
# tests/fixtures/git_root_vectors.tsv (also read by the Bash and Fish tests).
# See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/git_root_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

S=$(mktemp -d "${TMPDIR:-/tmp}/gpy-git-root.XXXXXX") || exit 1
trap 'rm -rf "$S"' EXIT
S=$(realpath "$S")

# S-relative path -> absolute; `.` is S itself, \e stays "none" (empty).
abs() {
    case $1 in
        '\e') ;;
        .) printf '%s' "$S" ;;
        *) printf '%s' "$S/$1" ;;
    esac
}

check() {
    if [[ "$3" == "$2" ]]; then
        pass "$1"
    else
        fail "$1: expected [$2], got [$3]"
    fi
}

vector_count=0
while IFS=$'\t' read -r kind c2 c3 c4 _rest; do
    [[ -z "$kind" || "$kind" == \#* ]] && continue
    case $kind in
        mkdir) mkdir -p "$S/$c2" ;;
        gitdir) mkdir -p "$S/$c2/.git" ;;
        gitfile) mkdir -p "$S/$c2" && printf 'gitdir: %s\n' "$S/.worktree-admin" > "$S/$c2/.git" ;;
        symlink) ln -s "$S/$c3" "$S/$c2" ;;
        root)
            vector_count=$((vector_count + 1))
            expected=$(abs "$c3")
            actual=$(__gpy_find_git_root "$S/$c2")
            git_status=$?
            [[ -n "$expected" || $git_status -ne 0 ]] || fail "root $c2: found nothing but returned 0"
            check "root vector $vector_count ($c2)" "$expected" "$actual"
            ;;
        key)
            vector_count=$((vector_count + 1))
            expected=$(abs "$c4")
            actual=""
            __gpy_instant_cache_key_path "$c2" "$S/$c3" actual
            git_status=$?
            [[ -n "$expected" || $git_status -ne 0 ]] || fail "key $c2 $c3: found nothing but returned 0"
            [[ -z "$expected" || $git_status -eq 0 ]] || fail "key $c2 $c3: found a key but returned $git_status"
            check "key vector $vector_count ($c2 $c3)" "$expected" "$actual"
            ;;
        *) fail "unknown row kind [$kind]" ;;
    esac
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count git-root vectors"
