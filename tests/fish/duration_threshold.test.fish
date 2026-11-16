#!/usr/bin/env fish
# Duration segment gating uses the agent-exported `__duration_threshold_ms`
# variable, never the retired `__gpy_duration_threshold`.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l repo_root (path dirname (path dirname (path dirname (status --current-filename))))
cd $repo_root

echo "Testing duration threshold variable..."

# Source the duration segment
source fish/segments/duration.fish

function test_duration_hidden_below_threshold
    echo "Test 1: segment hidden when CMD_DURATION is below threshold"
    set -g __duration_threshold_ms 500
    set -g CMD_DURATION 100

    if segment_duration_detect
        fail "segment should be hidden (100ms < 500ms threshold)"
        return 1
    end
    pass "segment hidden when below threshold"
end

function test_duration_visible_at_threshold
    echo "Test 2: segment visible when CMD_DURATION meets threshold"
    set -g __duration_threshold_ms 500
    set -g CMD_DURATION 500

    if not segment_duration_detect
        fail "segment should be visible (500ms >= 500ms threshold)"
        return 1
    end
    pass "segment visible at threshold"
end

function test_duration_visible_above_threshold
    echo "Test 3: segment visible when CMD_DURATION exceeds threshold"
    set -g __duration_threshold_ms 200
    set -g CMD_DURATION 1500

    if not segment_duration_detect
        fail "segment should be visible (1500ms > 200ms threshold)"
        return 1
    end
    pass "segment visible above threshold"
end

function test_legacy_variable_not_consulted
    echo "Test 4: legacy __gpy_duration_threshold is ignored"
    # Set legacy var to a high value that would suppress display,
    # and the correct var to a low value that allows display.
    set -g __gpy_duration_threshold 99999
    set -g __duration_threshold_ms 100
    set -g CMD_DURATION 500

    if not segment_duration_detect
        fail "segment should be visible — legacy var must not gate display"
        return 1
    end
    pass "legacy variable is not consulted"

    set -e __gpy_duration_threshold
end

function test_default_when_var_unset
    echo "Test 5: segment uses built-in default when __duration_threshold_ms is unset"
    set -e __duration_threshold_ms
    # Default is 100ms in fish (segment file fallback)
    set -g CMD_DURATION 200

    if not segment_duration_detect
        fail "segment should be visible with default threshold (200ms > 100ms default)"
        return 1
    end
    pass "default threshold applied when variable is unset"
end

function test_default_boundary
    echo "Test 6: the built-in default is 100 ms, the exported theme default"
    set -e __duration_threshold_ms
    set -g CMD_DURATION 99
    if segment_duration_detect
        fail "99 ms should be hidden by the 100 ms default"
        return 1
    end
    set -g CMD_DURATION 100
    if not segment_duration_detect
        fail "100 ms should be shown by the 100 ms default"
        return 1
    end
    pass "default threshold is 100 ms"
end

function test_idle_prompt_hides_previous_duration
    echo "Test 7: a prompt with no command since the last one hides the pill"
    set -g __duration_threshold_ms 100
    set -g CMD_DURATION 500
    # A command ran: fish_postexec, then the next prompt.
    emit fish_postexec
    emit fish_prompt
    if not segment_duration_detect
        fail "the prompt after a 500 ms command should show the pill"
        return 1
    end
    # Empty Enter or Ctrl-C: a new prompt with no fish_postexec, while Fish
    # keeps the old CMD_DURATION.
    emit fish_prompt
    if segment_duration_detect
        fail "an empty Enter should not show the previous command's duration again"
        return 1
    end
    emit fish_postexec
    emit fish_prompt
    if not segment_duration_detect
        fail "the next command should show its duration again"
        return 1
    end
    pass "only a prompt after a command shows its duration"
end

# Run all tests
test_duration_hidden_below_threshold
test_duration_visible_at_threshold
test_duration_visible_above_threshold
test_legacy_variable_not_consulted
test_default_when_var_unset
test_default_boundary
test_idle_prompt_hides_previous_duration

echo ""
test_finish "duration threshold"
