#!/usr/bin/env fish
# Test: segment_language_detect walks upward to find project markers

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
# segment_language_detect calls __gpy_memoized_git_root / __gpy_find_git_root
# (core/ipc.fish, #342). The real init sequence (core/init.fish) always
# sources ipc.fish before any segment, so segment_language_detect has never
# had to declare this dependency itself -- this test only worked previously
# because a developer's own config.fish happened to source the full fish/
# tree first, making the function available by accident (#630).
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/segments/language.fish"

set -g pass_count 0
set -g fail_count 0

function assert_detect --argument-names label expected_status dir
    set -l old_pwd $PWD
    cd $dir
    segment_language_detect
    set -l got $status
    cd $old_pwd
    if test $got -eq $expected_status
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected $expected_status, got $got)"
    end
end

# --- Setup: project with Cargo.toml ---
set -l proj (mktemp -d)
touch "$proj/Cargo.toml"
mkdir -p "$proj/src/nested/deep"

# Project root should be detected
assert_detect "detects at project root" 0 "$proj"

# Subdirectory should also be detected (upward walk)
assert_detect "detects in src/" 0 "$proj/src"
assert_detect "detects in deeply nested subdir" 0 "$proj/src/nested/deep"

# Plain temp directory with no markers should NOT be detected
set -l empty_dir (mktemp -d)
assert_detect "not detected in plain dir" 1 "$empty_dir"

# .git marker should also trigger detection
set -l git_proj (mktemp -d)
mkdir "$git_proj/.git"
mkdir -p "$git_proj/lib/sub"
assert_detect "detects via .git at root" 0 "$git_proj"
assert_detect "detects via .git in subdir" 0 "$git_proj/lib/sub"

# Cleanup
rm -rf "$proj" "$empty_dir" "$git_proj"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count language_detect subdir tests passed"
