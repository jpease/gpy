#!/usr/bin/env fish
# tests/fish/segments.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Functional tests for segment detection logic

echo "Testing GPY segment functions..."

source fish/core/init.fish
source fish/segments/git.fish
source fish/segments/language.fish
source fish/segments/directory.fish
source fish/segments/clock.fish

# Drop inherited repo-scoping git env so the worktree fixture below targets its
# own throwaway repo, not this one, when run from a git hook (#275).
set -e GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX GIT_NAMESPACE

# Isolate environment from user configuration
set -l saved_home $HOME
set -l saved_xdg_config $XDG_CONFIG_HOME
set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
mkdir -p $XDG_CONFIG_HOME/gpy

# Ensure language is enabled for tests (regardless of user config)
set -gx GPY_LANGUAGE_ENABLED 1

# Test 1: Git segment detects real git repository
echo "Test 1: Git detection in actual repository"
set -l temp_repo (mktemp -d)
cd $temp_repo
git init >/dev/null 2>&1
git config user.email "test@example.com"
git config user.name "Test User"
git config commit.gpgsign false

if segment_git_detect
    echo "✅ Git segment correctly detects git repository"
else
    echo "❌ Git segment failed to detect git repository"
    cd -
    rm -rf $temp_repo
    exit 1
end

cd -
rm -rf $temp_repo

# Test 2: Git segment detects linked worktrees with .git files
echo "Test 2: Git detection in linked worktree"
set -l worktree_base (mktemp -d)
set -l worktree_repo "$worktree_base/repo"
set -l worktree_checkout "$worktree_base/repo-worktree"
set -l worktree_hooks "$worktree_base/hooks"
mkdir -p "$worktree_repo"
mkdir -p "$worktree_hooks"
git -C "$worktree_repo" init >/dev/null 2>&1
git -C "$worktree_repo" symbolic-ref HEAD refs/heads/main
git -C "$worktree_repo" config user.email "test@example.com"
git -C "$worktree_repo" config user.name "Test User"
git -C "$worktree_repo" config commit.gpgsign false
git -C "$worktree_repo" config core.hooksPath "$worktree_hooks"
echo test >"$worktree_repo/file.txt"
git -C "$worktree_repo" add file.txt >/dev/null 2>&1
set -l worktree_tree (git -C "$worktree_repo" write-tree)
set -l worktree_commit (printf 'init\n' | git -C "$worktree_repo" commit-tree "$worktree_tree")
git -C "$worktree_repo" update-ref refs/heads/main "$worktree_commit"
git -c core.hooksPath="$worktree_hooks" -C "$worktree_repo" worktree add -q "$worktree_checkout"
cd "$worktree_checkout"

if segment_git_detect
    echo "✅ Git segment correctly detects linked worktree"
else
    echo "❌ Git segment failed to detect linked worktree"
    cd -
    rm -rf "$worktree_base"
    exit 1
end

cd -
rm -rf "$worktree_base"

# Test 3: Git segment returns nothing in non-git directory
echo "Test 3: Git detection in non-git directory"
set -l temp_dir (mktemp -d)
cd $temp_dir

if segment_git_detect
    echo "❌ Git segment should not detect non-git directory"
    cd -
    rm -rf $temp_dir
    exit 1
else
    echo "✅ Git segment correctly returns false for non-git directory"
end

cd -
rm -rf $temp_dir

# Test 4: Language segment detects directory with package.json
echo "Test 4: Language detection with package.json"
set -l temp_proj (mktemp -d)
cd $temp_proj
echo '{"name":"test"}' >package.json

if segment_language_detect
    echo "✅ Language segment correctly detects project with package.json"
else
    echo "❌ Language segment failed to detect package.json"
    cd -
    rm -rf $temp_proj
    exit 1
end

cd -
rm -rf $temp_proj

# Test 5: Language segment detects directory with Cargo.toml
echo "Test 5: Language detection with Cargo.toml"
set -l temp_cargo (mktemp -d)
cd $temp_cargo
echo '[package]' >Cargo.toml

if segment_language_detect
    echo "✅ Language segment correctly detects project with Cargo.toml"
else
    echo "❌ Language segment failed to detect Cargo.toml"
    cd -
    rm -rf $temp_cargo
    exit 1
end

cd -
rm -rf $temp_cargo

# Test 6: Language segment returns nothing without project files
echo "Test 6: Language detection in empty directory"
set -l temp_empty (mktemp -d)
cd $temp_empty

if segment_language_detect
    echo "❌ Language segment should not detect empty directory"
    cd -
    rm -rf $temp_empty
    exit 1
else
    echo "✅ Language segment correctly returns false for empty directory"
end

cd -
rm -rf $temp_empty

# Test 7: Directory segment always detects
echo "Test 7: Directory segment detection"
if segment_directory_detect
    echo "✅ Directory segment always returns true"
else
    echo "❌ Directory segment unexpectedly returned false"
    exit 1
end

# Test 8: Clock segment always detects
echo "Test 8: Clock segment detection"
if segment_clock_detect
    echo "✅ Clock segment always returns true"
else
    echo "❌ Clock segment unexpectedly returned false"
    exit 1
end

# Test 9: Verify render functions exist
echo "Test 9: Segment render functions"
if functions -q segment_git_render
    and functions -q segment_language_render
    and functions -q segment_directory_render
    and functions -q segment_clock_render
    echo "✅ All segment render functions are defined"
else
    echo "❌ Some segment render functions missing"
    exit 1
end

# Directory display modes and truncation are rendered by the agent, not by
# the shell, and are asserted end to end (real agent, real repo) in
# tests/fish/e2e_prompt_content.test.fish (#644).

# Restore environment and cleanup
rm -rf $temp_home
set -gx HOME $saved_home
if test -n "$saved_xdg_config"
    set -gx XDG_CONFIG_HOME $saved_xdg_config
else
    set -e XDG_CONFIG_HOME
end

echo "🎉 All segment tests passed!"
