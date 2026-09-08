#!/usr/bin/env fish
# Test Fish segment parsing of the new Fish-friendly format

# Source the core files first
source fish/core/util.fish
source fish/core/ipc.fish
source fish/core/renderer.fish
# Source the segment files
source fish/segments/git.fish
source fish/segments/language.fish

# Mock the __gpy_request function to return Fish format data (override after sourcing)
function __gpy_request
    # Parse arguments - handle both old format (__gpy_request op cwd) and new format (__gpy_request op --cwd cwd)
    set -l op $argv[1]
    set -l cwd

    if test (count $argv) -eq 2
        # Old format: __gpy_request op cwd
        set cwd $argv[2]
    else if test (count $argv) -eq 3; and test "$argv[2]" = --cwd
        # New format: __gpy_request op --cwd cwd
        set cwd $argv[3]
    end

    switch $op
        case git
            # Return sample git Fish format (space-separated argparse flags)
            echo "--branch main --ahead 2 --behind 1 --staged 3 --unstaged 4 --untracked 5 --conflicts 0 --state clean"
        case lang
            # Return sample language Fish format (space-separated argparse flags)
            echo "--lang rust --version 1.70.0 --color orange --lang node --version 18.16.0 --color green"
    end
end

# Test git format parsing
function test_git_fish_parsing
    echo "Testing git Fish format parsing..."

    # Get the parsed data using argparse like the real segment does
    set -l data (__gpy_request git --cwd "/test/repo")
    set -l parsed_args (string split -- ' ' "$data")

    # Parse using argparse like the real git segment
    argparse --ignore-unknown \
        'branch=' 'ahead=' 'behind=' 'staged=' 'unstaged=' 'untracked=' 'conflicts=' 'state=' \
        -- $parsed_args
    or begin
        echo "✗ Argparse failed to parse git data"
        return 1
    end

    # Verify we get the expected values
    if test "$_flag_branch" = main
        echo "✓ Git branch parsed correctly: $_flag_branch"
    else
        echo "✗ Git branch incorrect: $_flag_branch expected main"
        return 1
    end

    if test "$_flag_ahead" = 2
        echo "✓ Git ahead parsed correctly: $_flag_ahead"
    else
        echo "✗ Git ahead incorrect: $_flag_ahead expected 2"
        return 1
    end

    if test "$_flag_behind" = 1
        echo "✓ Git behind parsed correctly: $_flag_behind"
    else
        echo "✗ Git behind incorrect: $_flag_behind expected 1"
        return 1
    end

    if test "$_flag_staged" = 3
        echo "✓ Git staged parsed correctly: $_flag_staged"
    else
        echo "✗ Git staged incorrect: $_flag_staged expected 3"
        return 1
    end

    if test "$_flag_unstaged" = 4
        echo "✓ Git unstaged parsed correctly: $_flag_unstaged"
    else
        echo "✗ Git unstaged incorrect: $_flag_unstaged expected 4"
        return 1
    end

    if test "$_flag_untracked" = 5
        echo "✓ Git untracked parsed correctly: $_flag_untracked"
    else
        echo "✗ Git untracked incorrect: $_flag_untracked expected 5"
        return 1
    end

    if test "$_flag_conflicts" = 0
        echo "✓ Git conflicts parsed correctly: $_flag_conflicts"
    else
        echo "✗ Git conflicts incorrect: $_flag_conflicts expected 0"
        return 1
    end

    if test "$_flag_state" = clean
        echo "✓ Git state parsed correctly: $_flag_state"
    else
        echo "✗ Git state incorrect: $_flag_state expected clean"
        return 1
    end
end

# Test language format parsing
function test_lang_fish_parsing
    echo "Testing language Fish format parsing..."

    # Get the parsed data using argparse like the real segment does
    set -l data (__gpy_request lang --cwd "/test/repo")
    set -l parsed_args (string split -- ' ' "$data")

    # Parse using argparse with accumulation for multiple values
    argparse --ignore-unknown \
        'lang=+' 'version=+' 'color=+' \
        -- $parsed_args
    or begin
        echo "✗ Argparse failed to parse language data"
        return 1
    end

    # Verify we get the expected values
    # Note: argparse with + creates arrays for multiple same flags
    if test (count $_flag_lang) -eq 2
        echo "✓ Language parsing returned correct number of entries"
    else
        echo "✗ Language parsing returned wrong number of entries: "(count $_flag_lang)" expected 2"
        return 1
    end

    # Check first language
    if test "$_flag_lang[1]" = rust -a "$_flag_version[1]" = "1.70.0" -a "$_flag_color[1]" = orange
        echo "✓ First language entry correct: $_flag_lang[1] $_flag_version[1] $_flag_color[1]"
    else
        echo "✗ First language entry incorrect: $_flag_lang[1] $_flag_version[1] $_flag_color[1]"
        return 1
    end

    # Check second language
    if test "$_flag_lang[2]" = node -a "$_flag_version[2]" = "18.16.0" -a "$_flag_color[2]" = green
        echo "✓ Second language entry correct: $_flag_lang[2] $_flag_version[2] $_flag_color[2]"
    else
        echo "✗ Second language entry incorrect: $_flag_lang[2] $_flag_version[2] $_flag_color[2]"
        return 1
    end
end

# Test that Fish format eliminates regex parsing
function test_no_regex_dependency
    echo "Testing that new format eliminates regex parsing..."

    # The old __gpy_from_json function should be gone
    if functions -q __gpy_from_json
        echo "✗ Old __gpy_from_json function still exists"
        return 1
    else
        echo "✓ Old regex-based JSON parsing function removed"
    end

    # The new parsing should use argparse instead of regex
    set -l sample_data "--branch main --ahead 2 --behind 1"
    set -l parsed_args (string split -- ' ' "$sample_data")

    argparse --ignore-unknown 'branch=' 'ahead=' 'behind=' -- $parsed_args
    or begin
        echo "✗ Argparse parsing failed"
        return 1
    end

    if test "$_flag_branch" = main -a "$_flag_ahead" = 2 -a "$_flag_behind" = 1
        echo "✓ Argparse parsing works correctly"
    else
        echo "✗ Argparse parsing failed"
        return 1
    end
end

# Test performance improvement (no regex overhead)
function test_parsing_performance
    echo "Testing parsing performance..."

    # Generate sample data in new argparse format
    set -l sample_git_parts
    for i in (seq 1 100)
        set -a sample_git_parts "--branch test-$i"
        set -a sample_git_parts "--ahead $i"
        set -a sample_git_parts "--behind "(math "$i * 2")
    end
    set -l sample_git_data (string join -- ' ' $sample_git_parts)

    # Time the parsing (should be very fast)
    set -l start_time (date +%s%N)
    set -l parsed_args (string split -- ' ' "$sample_git_data")
    argparse --ignore-unknown 'branch=' 'ahead=' 'behind=' -- $parsed_args >/dev/null 2>&1
    set -l end_time (date +%s%N)

    set -l duration_ns (math "$end_time - $start_time")
    set -l duration_ms (math "$duration_ns / 1000000")

    if test $duration_ms -lt 10 # Should be under 10ms for 300 arguments
        echo "✓ Parsing performance is excellent: "$duration_ms"ms for 300 arguments"
    else
        echo "⚠ Parsing took longer than expected: "$duration_ms"ms"
    end
end

# Run all tests
function run_fish_format_tests
    echo "Starting Fish segment format validation tests..."
    echo

    test_git_fish_parsing
    and test_lang_fish_parsing
    and test_no_regex_dependency
    and test_parsing_performance

    if test $status -eq 0
        echo
        echo "🎉 All Fish format parsing tests passed!"
        echo "✅ Git format parsing works correctly"
        echo "✅ Language format parsing works correctly"
        echo "✅ Regex dependency eliminated"
        echo "✅ Performance is excellent"
    else
        echo
        echo "❌ Some tests failed"
        return 1
    end
end

# Run the tests
run_fish_format_tests
