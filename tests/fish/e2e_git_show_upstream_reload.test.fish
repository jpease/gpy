#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Git.show_upstream Hot-Reload
#
# This test verifies that git.show_upstream config changes hot-reload across:
#   1. CLI oneshot git command (parses JSON to verify ahead/behind behavior)
#   2. Fish prompt rendering (the theme export carries no GPY_GIT_SHOW_UPSTREAM)
#
# Test setup:
#   - Creates bare repo as "origin"
#   - Clones it to create working repo with upstream tracking
#   - Makes local commits to create ahead/behind divergence
#   - Toggles show_upstream and verifies actual calculation changes
#
# Expected runtime: ~10 seconds

source (dirname (status -f))/../lib/test_helpers.fish

function print_banner
    print_test_header "E2E Test: Git Show Upstream Reload"
end

function parse_json_field --argument-names json_output field_name
    # Extract a field from JSON output
    # This is a simple parser that looks for "field": value
    echo "$json_output" | grep -o "\"$field_name\":[^,}]*" | sed 's/.*://' | tr -d ' '
end

# Poll predicate: oneshot git for $repo reports the expected ahead count, i.e.
# a config hot-reload has landed.
function __oneshot_ahead_is --argument-names repo want
    set -l json (gpy-agent oneshot git --cwd "$repo" --format json 2>/dev/null)
    test (parse_json_field "$json" "ahead") = "$want"
end

function test_git_show_upstream_reload
    print_banner
    init_test_env

    # Setup test config directory
    set -l config_root $__gpy_test_tmp_dir/config
    set -l config_dir "$config_root/gpy"
    mkdir -p "$config_dir"
    set -l config_path "$config_dir/config.toml"

    # Write initial config with show_upstream = true
    printf "[ui]\ntheme = \"default\"\nshow_icons = false\n\n[git]\nenabled = true\nshow_upstream = true\n" >"$config_path"

    # Set environment for test
    set -gx XDG_CONFIG_HOME "$config_root"
    set -gx PATH "$PWD/gpy-agent/target/debug" $PATH

    # Create bare repo to act as "origin"
    set -l bare_repo "$__gpy_test_tmp_dir/bare.git"
    mkdir -p "$bare_repo"
    cd "$bare_repo"
    git init --bare >/dev/null 2>&1

    # Clone bare repo to create working repo
    set -l work_repo "$__gpy_test_tmp_dir/work-repo"
    cd "$__gpy_test_tmp_dir"
    git clone "$bare_repo" "$work_repo" >/dev/null 2>&1

    cd "$work_repo"
    git config user.email "test@example.com"
    git config user.name Test
    git config commit.gpgsign false

    # Create initial commit and push to origin
    echo initial >file.txt
    git add file.txt
    git commit -m "Initial commit" >/dev/null 2>&1
    git branch -M main
    git push -u origin main >/dev/null 2>&1

    # Create local commits (ahead of origin)
    echo "local change 1" >>file.txt
    git add file.txt
    git commit -m "Local commit 1" >/dev/null 2>&1

    echo "local change 2" >>file.txt
    git add file.txt
    git commit -m "Local commit 2" >/dev/null 2>&1

    # Verify we have upstream configured
    set -l upstream (git rev-parse --abbrev-ref '@{upstream}' 2>/dev/null)
    if test "$upstream" != origin/main
        fail "Upstream setup" "Expected origin/main, got: $upstream"
        cleanup_test_files
        return 1
    end
    pass "Upstream setup"

    # Start agent
    if start_test_agent
        pass "Agent Start"
    else
        fail "Agent Start" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    # Wait for the agent to load config and compute upstream.
    poll_until 5 __oneshot_ahead_is "$work_repo" 2

    # Verify show_upstream = true calculates ahead/behind
    set -l output1 (gpy-agent oneshot git --cwd "$work_repo" --format json 2>/dev/null)

    if test -z "$output1"
        fail "Oneshot git with show_upstream=true" "No JSON output"
        cleanup_test_files
        return 1
    end

    # Parse ahead count from JSON
    set -l ahead1 (parse_json_field "$output1" "ahead")

    # We made 2 local commits, so ahead should be 2
    if test "$ahead1" = 2
        pass "Oneshot shows ahead=2 with show_upstream=true"
    else
        fail "Oneshot shows ahead=2 with show_upstream=true" "Expected ahead=2, got: $ahead1"
        echo "Full JSON: $output1"
        cleanup_test_files
        return 1
    end

    # Verify theme export does not export GPY_GIT_SHOW_UPSTREAM (config values are not environment mirrors)
    set -l theme1 (gpy-agent theme export --shell fish 2>/dev/null)

    if not echo "$theme1" | grep -q GPY_GIT_SHOW_UPSTREAM
        pass "Theme export does not export GPY_GIT_SHOW_UPSTREAM"
    else
        fail "Theme export does not export GPY_GIT_SHOW_UPSTREAM" "Variable present in the export"
        cleanup_test_files
        return 1
    end

    # Change config to show_upstream = false
    printf "[ui]\ntheme = \"default\"\nshow_icons = false\n\n[git]\nenabled = true\nshow_upstream = false\n" >"$config_path"

    # Wait for the reload to land: ahead drops to 0 when upstream is skipped.
    poll_until 6 __oneshot_ahead_is "$work_repo" 0

    # Verify oneshot still returns valid output (but may skip upstream calculation internally)
    set -l output2 (gpy-agent oneshot git --cwd "$work_repo" --format json 2>/dev/null)

    if test -z "$output2"
        fail "Oneshot git with show_upstream=false" "No JSON output after config change"
        cleanup_test_files
        return 1
    end

    # When show_upstream is false, ahead/behind should be 0 (skipped calculation)
    set -l ahead2 (parse_json_field "$output2" "ahead")
    set -l behind2 (parse_json_field "$output2" "behind")

    if test "$ahead2" = 0 -a "$behind2" = 0
        pass "Oneshot shows ahead=0,behind=0 with show_upstream=false"
    else
        fail "Oneshot shows ahead=0,behind=0 with show_upstream=false" "Expected 0,0 got: $ahead2,$behind2"
        echo "Full JSON: $output2"
        cleanup_test_files
        return 1
    end

    # Verify the theme export still does not export GPY_GIT_SHOW_UPSTREAM
    set -l theme2 (gpy-agent theme export --shell fish 2>/dev/null)

    if not echo "$theme2" | grep -q GPY_GIT_SHOW_UPSTREAM
        pass "Theme export does not export GPY_GIT_SHOW_UPSTREAM"
    else
        fail "Theme export does not export GPY_GIT_SHOW_UPSTREAM" "Variable present in the export"
        cleanup_test_files
        return 1
    end

    # Re-enable show_upstream and verify it hot-reloads
    printf "[ui]\ntheme = \"default\"\nshow_icons = false\n\n[git]\nenabled = true\nshow_upstream = true\n" >"$config_path"

    # Wait for the reload to land: ahead returns to 2 once upstream is re-enabled.
    poll_until 6 __oneshot_ahead_is "$work_repo" 2

    # Verify upstream calculation is re-enabled
    set -l output3 (gpy-agent oneshot git --cwd "$work_repo" --format json 2>/dev/null)
    set -l ahead3 (parse_json_field "$output3" "ahead")

    if test "$ahead3" = 2
        pass "Oneshot shows ahead=2 after re-enabling upstream"
    else
        fail "Oneshot shows ahead=2 after re-enabling upstream" "Expected ahead=2, got: $ahead3"
        cleanup_test_files
        return 1
    end

    # Cleanup
    stop_test_agent
    cleanup_test_files

    return 0
end

# Run the test
test_git_show_upstream_reload; or exit 1
test_finish "git show upstream reload"
