#!/usr/bin/env fish
# Test segment override toggles (GPY_MINIMAL_SEGMENTS and GPY_TEST_SEGMENTS)

echo "Testing segment override toggles..."

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

# Source modules needed
source fish/core/ipc.fish

function test_minimal_segments_loads_only_directory
    echo "Test 1: GPY_MINIMAL_SEGMENTS loads only directory"

    # Clean state
    set -e __enabled_segments
    set -e GPY_TEST_SEGMENTS
    set -e GPY_SHOW_LANGUAGES

    # Set GPY_MINIMAL_SEGMENTS to enable minimal mode BEFORE sourcing
    set -gx GPY_MINIMAL_SEGMENTS "1"

    # Source core/ipc.fish and core/util.fish to get dependencies
    # but mock the side-effect functions before sourcing init.fish
    function __gpy_load_theme
        # No-op - avoid calling gpy-agent
    end

    function __gpy_load_default_config
        # Set the default segments (mimics what the real function does)
        set -g __enabled_segments clock duration language directory git
    end

    function __gpy_load_segments
        # No-op - we don't want to actually source segment files
    end

    # Source the real init.fish to test the actual production logic
    # This will execute the segment selection code and apply the override
    source fish/core/init.fish

    # Verify only directory segment is enabled
    if test (count $__enabled_segments) -eq 1
        echo "✅ Minimal segments contains only 1 segment"
    else
        echo "❌ Expected 1 segment, got "(count $__enabled_segments)
        return 1
    end

    if test "$__enabled_segments[1]" = "directory"
        echo "✅ First segment is directory"
    else
        echo "❌ Expected directory, got $__enabled_segments[1]"
        return 1
    end

    # Clean up
    set -e GPY_MINIMAL_SEGMENTS
end

function test_gpy_test_segments_splits_correctly
    echo ""
    echo "Test 2: GPY_TEST_SEGMENTS splits correctly"

    # Clean state
    set -e __enabled_segments
    set -e GPY_MINIMAL_SEGMENTS
    set -e GPY_SHOW_LANGUAGES

    # Set custom segment list BEFORE sourcing
    set -gx GPY_TEST_SEGMENTS "clock git"

    # Mock functions that would cause side effects
    function __gpy_load_theme
        # No-op
    end

    function __gpy_load_default_config
        set -g __enabled_segments clock duration language directory git
    end

    function __gpy_load_segments
        # No-op
    end

    # Source the real init.fish
    source fish/core/init.fish

    # Verify segments match GPY_TEST_SEGMENTS
    if test (count $__enabled_segments) -eq 2
        echo "✅ Test segments count is 2"
    else
        echo "❌ Expected 2 segments, got "(count $__enabled_segments)
        return 1
    end

    if test "$__enabled_segments[1]" = "clock"
        echo "✅ First segment is clock"
    else
        echo "❌ Expected clock, got $__enabled_segments[1]"
        return 1
    end

    if test "$__enabled_segments[2]" = "git"
        echo "✅ Second segment is git"
    else
        echo "❌ Expected git, got $__enabled_segments[2]"
        return 1
    end

    # Clean up
    set -e GPY_TEST_SEGMENTS
end

function test_gpy_show_languages_zero_removes_language
    echo ""
    echo "Test 3: GPY_SHOW_LANGUAGES=0 removes language"

    # Clean state
    set -e __enabled_segments
    set -e GPY_MINIMAL_SEGMENTS
    set -e GPY_TEST_SEGMENTS

    # Set GPY_SHOW_LANGUAGES to 0 BEFORE sourcing
    set -gx GPY_SHOW_LANGUAGES "0"

    # Mock functions
    function __gpy_load_theme
        # No-op
    end

    function __gpy_load_default_config
        set -g __enabled_segments clock duration language directory git
    end

    function __gpy_load_segments
        # No-op
    end

    # Source the real init.fish
    source fish/core/init.fish

    # Verify language is not in the list
    if test (count $__enabled_segments) -eq 4
        echo "✅ GPY_SHOW_LANGUAGES=0 results in 4 segments"
    else
        echo "❌ Expected 4 segments, got "(count $__enabled_segments)
        return 1
    end

    if string match -q language $__enabled_segments
        echo "❌ Language segment should not be present"
        return 1
    else
        echo "✅ Language not in segments"
    end

    # Clean up
    set -e GPY_SHOW_LANGUAGES
end

function test_minimal_overrides_show_languages
    echo ""
    echo "Test 4: GPY_MINIMAL_SEGMENTS overrides GPY_SHOW_LANGUAGES"

    # Clean state
    set -e __enabled_segments
    set -e GPY_TEST_SEGMENTS

    # Set both GPY_SHOW_LANGUAGES and GPY_MINIMAL_SEGMENTS BEFORE sourcing
    set -gx GPY_SHOW_LANGUAGES "0"
    set -gx GPY_MINIMAL_SEGMENTS "1"

    # Mock functions
    function __gpy_load_theme
        # No-op
    end

    function __gpy_load_default_config
        set -g __enabled_segments clock duration language directory git
    end

    function __gpy_load_segments
        # No-op
    end

    # Source the real init.fish
    source fish/core/init.fish

    # Verify GPY_MINIMAL_SEGMENTS takes precedence
    if test (count $__enabled_segments) -eq 1
        echo "✅ Minimal overrides SHOW_LANGUAGES (1 segment)"
    else
        echo "❌ Expected 1 segment, got "(count $__enabled_segments)
        return 1
    end

    if test "$__enabled_segments[1]" = "directory"
        echo "✅ Only directory segment present"
    else
        echo "❌ Expected directory, got $__enabled_segments[1]"
        return 1
    end

    # Clean up
    set -e GPY_SHOW_LANGUAGES
    set -e GPY_MINIMAL_SEGMENTS
end

# Run all tests
test_minimal_segments_loads_only_directory
or exit 1

test_gpy_test_segments_splits_correctly
or exit 1

test_gpy_show_languages_zero_removes_language
or exit 1

test_minimal_overrides_show_languages
or exit 1

echo ""
echo "🎉 All segment toggle tests passed!"
