#!/usr/bin/env fish
# Tests that duration segment gating uses the agent-exported `__duration_threshold_ms`
# variable (not the legacy `__gpy_duration_threshold`). Regression for #203.

set -l repo_root (path dirname (path dirname (path dirname (status --current-filename))))
cd $repo_root

echo "Testing duration threshold variable (#203)..."

# Source the duration segment
source fish/segments/duration.fish

function test_duration_hidden_below_threshold
    echo "Test 1: segment hidden when CMD_DURATION is below threshold"
    set -g __duration_threshold_ms 500
    set -g CMD_DURATION 100

    if segment_duration_detect
        echo "❌ segment should be hidden (100ms < 500ms threshold)"
        return 1
    end
    echo "✅ segment hidden when below threshold"
end

function test_duration_visible_at_threshold
    echo "Test 2: segment visible when CMD_DURATION meets threshold"
    set -g __duration_threshold_ms 500
    set -g CMD_DURATION 500

    if not segment_duration_detect
        echo "❌ segment should be visible (500ms >= 500ms threshold)"
        return 1
    end
    echo "✅ segment visible at threshold"
end

function test_duration_visible_above_threshold
    echo "Test 3: segment visible when CMD_DURATION exceeds threshold"
    set -g __duration_threshold_ms 200
    set -g CMD_DURATION 1500

    if not segment_duration_detect
        echo "❌ segment should be visible (1500ms > 200ms threshold)"
        return 1
    end
    echo "✅ segment visible above threshold"
end

function test_legacy_variable_not_consulted
    echo "Test 4: legacy __gpy_duration_threshold is ignored"
    # Set legacy var to a high value that would suppress display,
    # and the correct var to a low value that allows display.
    set -g __gpy_duration_threshold 99999
    set -g __duration_threshold_ms 100
    set -g CMD_DURATION 500

    if not segment_duration_detect
        echo "❌ segment should be visible — legacy var must not gate display"
        return 1
    end
    echo "✅ legacy variable is not consulted"

    set -e __gpy_duration_threshold
end

function test_default_when_var_unset
    echo "Test 5: segment uses built-in default when __duration_threshold_ms is unset"
    set -e __duration_threshold_ms
    # Default is 100ms in fish (segment file fallback)
    set -g CMD_DURATION 200

    if not segment_duration_detect
        echo "❌ segment should be visible with default threshold (200ms > 100ms default)"
        return 1
    end
    echo "✅ default threshold applied when variable is unset"
end

# Run all tests
set -l failures 0

test_duration_hidden_below_threshold; or set failures (math $failures + 1)
test_duration_visible_at_threshold; or set failures (math $failures + 1)
test_duration_visible_above_threshold; or set failures (math $failures + 1)
test_legacy_variable_not_consulted; or set failures (math $failures + 1)
test_default_when_var_unset; or set failures (math $failures + 1)

# Also verify bash/zsh segment files do not reference the old variable name
echo ""
echo "Test 6: bash/zsh segment files use __duration_threshold_ms (not legacy name)"
if grep -q __gpy_duration_threshold bash/segments/duration.bash zsh/segments/duration.zsh 2>/dev/null
    echo "❌ Found legacy __gpy_duration_threshold in bash/zsh segments"
    set failures (math $failures + 1)
else
    echo "✅ bash/zsh segments use __duration_threshold_ms"
end

echo ""
if test $failures -eq 0
    echo "All duration threshold tests passed ✅"
    exit 0
else
    echo "$failures test(s) failed ❌"
    exit 1
end
