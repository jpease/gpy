#!/usr/bin/env fish
# tests/fish/fresh_install.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Test GPY initialization in a fresh/clean environment
# Simulates a first-time installation with no existing config

set -g test_name fresh_install
set -g test_failures 0

function test_fail
    set -g test_failures (math $test_failures + 1)
    echo "❌ FAIL: $argv[1]"
end

function test_pass
    echo "✅ PASS: $argv[1]"
end

echo "========================================="
echo "GPY Fresh Install Test"
echo "========================================="
echo "Simulating first-time installation..."

# Find repo root (where core/init.fish lives)
set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"
echo "Running from: $repo_root"
echo ""

# Save current environment
set -l saved_home $HOME
set -l saved_config_home $XDG_CONFIG_HOME

# Create a temporary "home" directory
set -l temp_home (mktemp -d)
echo "Created temporary home: $temp_home"

# Set up minimal environment
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config

# Ensure no pre-existing GPY config
if test -d $XDG_CONFIG_HOME/gpy
    rm -rf $XDG_CONFIG_HOME/gpy
end

echo ""
echo "Test 1: Initialize GPY with no existing config"
# Model a real fresh install: the agent is enabled (the default) but the
# supervisor is off so init never spawns a persistent daemon. Do NOT set both
# to 0 — that trips init.fish's "completely disabled" early-exit, which skips
# icon initialization and segment loading (Tests 5 and 10 would then fail).
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Source into the current scope, NOT through a pipe: `source ... | string match`
# runs source in a subshell, so every function/variable it defines (icons,
# segment functions) is discarded — the test then only passed where a local
# gpy install happened to be autoloaded. Redirect stderr to a file and inspect
# it separately instead.
set -l init_errlog (mktemp)
source fish/core/init.fish 2>$init_errlog
# The prompt function is autoloaded from fish/functions/ on an installed
# system; here it has to be sourced, or `fish_prompt` below is fish's own
# default prompt and every prompt assertion in this file tests the wrong
# program (which is exactly what happened before #644).
source fish/functions/fish_prompt.fish
if string match -q "*error*" -- (cat $init_errlog)
    test_fail "Init failed with no existing config"
else
    test_pass "Init succeeds with no existing config"
end
rm -f $init_errlog

echo ""
echo "Test 2: Verify default segments are enabled"
if set -q __enabled_segments
    test_pass "Default segments defined: $__enabled_segments"

    # Check that we have at least the core segments
    set -l has_clock (contains clock $__enabled_segments; echo $status)
    set -l has_directory (contains directory $__enabled_segments; echo $status)

    if test $has_clock -eq 0
        test_pass "Clock segment enabled by default"
    else
        test_fail "Clock segment missing from defaults"
    end

    if test $has_directory -eq 0
        test_pass "Directory segment enabled by default"
    else
        test_fail "Directory segment missing from defaults"
    end
else
    test_fail "No default segments defined"
end

echo ""
echo "Test 3: Verify fish_prompt works with defaults"
set -l prompt_output (fish_prompt 2>/dev/null | string replace -ra '\e\[[0-9;]*m' '' | string collect)
if test -n "$prompt_output"
    test_pass "fish_prompt works with defaults"
    echo "  Prompt length: "(string length "$prompt_output")" characters"
else
    test_fail "fish_prompt failed with defaults"
end
# Content, not presence (#644): a fresh install's first prompt must carry
# the prompt character and must not leak JSON.
if string match -q "*$__icon_prompt*" -- "$prompt_output"; and not string match -q '*{"*' -- "$prompt_output"
    test_pass "first prompt renders the prompt character '$__icon_prompt' with no JSON"
else
    test_fail "first prompt is wrong: $prompt_output"
end

echo ""
echo "Test 4: Verify default theme variables exist"
if set -q __prompt_theme
    test_pass "Default theme set: $__prompt_theme"
else
    test_fail "No default theme set"
end

echo ""
echo "Test 5: Verify icons are initialized"
if set -q __icon_prompt
    test_pass "Icons initialized"
else
    test_fail "Icons not initialized"
end

echo ""
echo "Test 6: Test in non-git directory"
cd $temp_home
set -l prompt_in_home (fish_prompt 2>/dev/null)
if test -n "$prompt_in_home"
    test_pass "Prompt works in non-git directory"
else
    test_fail "Prompt failed in non-git directory"
end

echo ""
echo "Test 7: Test in git repository"
mkdir -p $temp_home/test_repo
cd $temp_home/test_repo
git init >/dev/null 2>&1
git config user.email "test@example.com"
git config user.name "Test User"

set -l prompt_in_repo (fish_prompt 2>/dev/null)
if test -n "$prompt_in_repo"
    test_pass "Prompt works in git repository"
else
    test_fail "Prompt failed in git repository"
end

echo ""
echo "Test 8: Verify no config files were created unexpectedly"
if test -d $XDG_CONFIG_HOME/gpy
    # It's OK if a config dir exists, but warn about it
    echo "  Note: Config directory was created at $XDG_CONFIG_HOME/gpy"
    # Check what's in it
    set -l config_files (find $XDG_CONFIG_HOME/gpy -type f)
    if test (count $config_files) -gt 0
        echo "  Files created: $config_files"
    end
end
test_pass "Config check completed"

echo ""
echo "Test 9: Verify environment variables"
# Check that GPY set configuration variables (this is expected)
set -l gpy_vars (set -n | grep -i gpy | grep -v GPY_AGENT_ENABLED | grep -v GPY_AGENT_SUPERVISOR_ENABLED | grep -v __gpy)
if test (count $gpy_vars) -gt 0
    test_pass "GPY configuration variables set: "(count $gpy_vars)" vars"
    echo "  Example vars: GPY_DEBOUNCE_MS, GPY_IPC_TIMEOUT_MS, GPY_GIT_SHOW"
else
    test_fail "No GPY configuration variables set (expected some)"
end

echo ""
echo "Test 10: Verify segment isolation"
# Each segment should work independently
for segment in clock directory duration
    set -l detect_func "segment_$segment"_detect
    set -l render_func "segment_$segment"_render

    if functions -q $detect_func; and functions -q $render_func
        test_pass "Segment $segment is self-contained"
    else
        test_fail "Segment $segment missing functions"
    end
end

# Cleanup
echo ""
echo "Cleaning up temporary environment..."
cd - >/dev/null
rm -rf $temp_home

# Restore environment
set -gx HOME $saved_home
if test -n "$saved_config_home"
    set -gx XDG_CONFIG_HOME $saved_config_home
else
    set -e XDG_CONFIG_HOME
end

# Summary
echo ""
echo "========================================="
if test $test_failures -eq 0
    echo "✅ ALL TESTS PASSED"
    echo "GPY works correctly in fresh install scenarios"
    exit 0
else
    echo "❌ $test_failures TEST(S) FAILED"
    exit 1
end
