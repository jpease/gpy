#!/usr/bin/env fish
# tests/fish/git_root_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_find_git_root and __gpy_instant_cache_key_path find the same
# directory as Bash's and Zsh's for every row of
# tests/fixtures/git_root_vectors.tsv (also read by the Bash and Zsh tests).
# See the fixture's header for the format.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish

set -l fixture "$repo_root/tests/fixtures/git_root_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end

set -g S (path resolve (mktemp -d (set -q TMPDIR; and echo $TMPDIR; or echo /tmp)/gpy-git-root.XXXXXX))
function __cleanup_scratch --on-event fish_exit
    rm -rf $S
end

# S-relative path -> absolute; `.` is S itself, \e stays "none" (empty).
function abs --argument-names rel
    switch $rel
        case '\e'
        case .
            printf '%s' $S
        case '*'
            printf '%s' "$S/$rel"
    end
end

set -l vector_count 0
while read -d \t -la cols
    test -z "$cols[1]"; and continue
    test (string sub -l 1 -- "$cols[1]") = '#'; and continue

    switch $cols[1]
        case mkdir
            mkdir -p "$S/$cols[2]"
        case gitdir
            mkdir -p "$S/$cols[2]/.git"
        case gitfile
            mkdir -p "$S/$cols[2]"; and printf 'gitdir: %s\n' "$S/.worktree-admin" >"$S/$cols[2]/.git"
        case symlink
            ln -s "$S/$cols[3]" "$S/$cols[2]"
        case root
            set vector_count (math $vector_count + 1)
            set -l expected (abs $cols[3])
            set -l actual (__gpy_find_git_root "$S/$cols[2]")
            set -l git_status $status
            test -n "$expected" -o $git_status -ne 0; or fail "root $cols[2]: found nothing but returned 0"
            check "root vector $vector_count ($cols[2])" "$expected" "$actual"
        case key
            set vector_count (math $vector_count + 1)
            set -l expected (abs $cols[4])
            set -l actual (__gpy_instant_cache_key_path $cols[2] "$S/$cols[3]")
            set -l git_status $status
            test -n "$expected" -o $git_status -ne 0; or fail "key $cols[2] $cols[3]: found nothing but returned 0"
            test -z "$expected" -o $git_status -eq 0; or fail "key $cols[2] $cols[3]: found a key but returned $git_status"
            check "key vector $vector_count ($cols[2] $cols[3])" "$expected" "$actual"
        case '*'
            fail "unknown row kind [$cols[1]]"
    end
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

test_finish "git-root vector tests"
