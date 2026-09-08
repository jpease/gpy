#!/usr/bin/env fish
# tests/fish/init_integration.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Integration test for core/init.fish - ensures the full initialization flow works
# and all enabled segments are properly registered.

set -g test_name init_integration
set -g test_failures 0

function test_fail
    set -g test_failures (math $test_failures + 1)
    echo "❌ FAIL: $argv[1]"
end

function test_pass
    echo "✅ PASS: $argv[1]"
end

function test_assert
    if test $argv[1] $argv[2] $argv[3]
        test_pass "$argv[4]"
    else
        test_fail "$argv[4]"
    end
end

echo "========================================="
echo "GPY Init Integration Test"
echo "========================================="

# Find repo root (where core/init.fish lives)
set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"
echo "Running from: $repo_root"
echo ""

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

# Test 1: Source core/init.fish without errors
echo "Test 1: Source core/init.fish"
if source fish/core/init.fish 2>&1 | string match -q "*error*"
    test_fail "core/init.fish produced errors"
else
    test_pass "core/init.fish loaded without errors"
end
# The prompt function is autoloaded from fish/functions/ on an installed
# system; here it has to be sourced, or `fish_prompt` below is fish's own
# default prompt and Test 6 measures the wrong program (which is exactly what
# it did before #644).
source fish/functions/fish_prompt.fish

# Test 2: Check that __enabled_segments is defined
echo ""
echo "Test 2: Check __enabled_segments is defined"
if set -q __enabled_segments
    test_pass "__enabled_segments is defined"
    echo "  Enabled segments: $__enabled_segments"
else
    test_fail "__enabled_segments is not defined"
end

# Test 3: Verify all enabled segments have detect functions
echo ""
echo "Test 3: Verify segment detect functions exist"
for segment in $__enabled_segments
    set -l detect_func "segment_$segment"_detect
    if functions -q $detect_func
        test_pass "segment_$segment"_detect" exists"
    else
        test_fail "segment_$segment"_detect" missing"
    end
end

# Test 4: Verify all enabled segments have render functions
echo ""
echo "Test 4: Verify segment render functions exist"
for segment in $__enabled_segments
    set -l render_func "segment_$segment"_render
    if functions -q $render_func
        test_pass "segment_$segment"_render" exists"
    else
        test_fail "segment_$segment"_render" missing"
    end
end

# Test 5: Check fish_prompt function exists
echo ""
echo "Test 5: Check fish_prompt function"
if functions -q fish_prompt
    test_pass "fish_prompt function exists"
else
    test_fail "fish_prompt function missing"
end

# Test 6: Verify fish_prompt returns non-empty output
echo ""
echo "Test 6: Verify fish_prompt output"
set -l prompt_output (fish_prompt 2>/dev/null | string replace -ra '\e\[[0-9;]*m' '' | string collect)
if test -n "$prompt_output"
    test_pass "fish_prompt returns non-empty output"
    echo "  Prompt length: "(string length "$prompt_output")" characters"
else
    test_fail "fish_prompt returned empty output"
end
# Content, not presence (#644): the prompt character is shell-rendered and
# must always be there; agent-rendered segments are covered by
# e2e_prompt_content.test.fish.
if string match -q "*$__icon_prompt*" -- "$prompt_output"
    test_pass "fish_prompt renders the prompt character '$__icon_prompt'"
else
    test_fail "fish_prompt output lacks the prompt character '$__icon_prompt': $prompt_output"
end

# Test 7: Verify theme variables are exported
echo ""
echo "Test 7: Verify theme variables exist"
set -l required_theme_vars \
    __prompt_base_bg \
    __prompt_base_fg \
    __icon_powerline_segment_start \
    __icon_powerline_segment_end

for var in $required_theme_vars
    if set -q $var
        test_pass "Theme variable $var is defined"
    else
        test_fail "Theme variable $var is missing"
    end
end

# Test 8: Verify icon variables are set
echo ""
echo "Test 8: Verify icon variables"
# NOTE: the duration segment is agent-rendered (#199) and no longer reads an
# `__icon_duration` constant, so it is intentionally not asserted here.
set -l required_icons \
    __icon_status_ok \
    __icon_status_fail \
    __icon_prompt

for icon in $required_icons
    if set -q $icon
        test_pass "Icon variable $icon is defined"
    else
        test_fail "Icon variable $icon is missing"
    end
end

# Test 9: Verify GPY config paths are set
echo ""
echo "Test 9: Verify GPY configuration"
if set -q __gpy_core_dir
    test_pass "__gpy_core_dir is defined: $__gpy_core_dir"
else
    test_fail "__gpy_core_dir is not defined"
end

# Test 10: Verify segment-specific functions work in a git repo
echo ""
echo "Test 10: Test segment functions in git repository"
set -l temp_repo (mktemp -d)
cd $temp_repo
git init >/dev/null 2>&1
git config user.email "test@example.com"
git config user.name "Test User"

if segment_git_detect
    test_pass "segment_git_detect works in git repository"
else
    test_fail "segment_git_detect failed in git repository"
end

# Render should work even with no commits
set -l git_output (segment_git_render 2>/dev/null)
# Git output might be empty for fresh repo, but function should not error
if test $status -eq 0
    test_pass "segment_git_render executes without error"
else
    test_fail "segment_git_render produced error"
end

cd -
rm -rf $temp_repo

# Test 11: Verify prompt contains all enabled segments (in a test environment)
echo ""
echo "Test 11: Verify prompt segments appear"
set -l prompt_output (fish_prompt 2>/dev/null)

# Check that prompt is not just the icon (regression test for "only clock renders")
set -l prompt_len (string length "$prompt_output")
if test $prompt_len -gt 10
    test_pass "Prompt has substantial content (not just icon): $prompt_len chars"
else
    test_fail "Prompt seems too short: $prompt_len chars"
end

# Test 12: Verify core utility functions exist
echo ""
echo "Test 12: Verify core utility functions"
set -l required_functions \
    __gpy_log_error \
    __gpy_load_default_config \
    __gpy_initialize_icons

for func in $required_functions
    if functions -q $func
        test_pass "Utility function $func exists"
    else
        test_fail "Utility function $func missing"
    end
end

# Summary
echo ""
echo "========================================="
if test $test_failures -eq 0
    echo "✅ ALL TESTS PASSED"
    exit 0
else
    echo "❌ $test_failures TEST(S) FAILED"
    exit 1
end
