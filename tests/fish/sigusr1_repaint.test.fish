#!/usr/bin/env fish
# Test for SIGUSR1 repaint mechanism
#
# This test validates the variable-based repaint pattern that enables
# live prompt updates. This pattern is critical - see docs/async-terminal-updates.md
# for the full explanation of what works and what doesn't.
#
# WHAT WE CAN TEST (automated):
# ✅ Function existence and registration
# ✅ Variable-based event pattern works
# ✅ Handler increments trigger variable
# ✅ Regression check: NOT using broken patterns (emit fish_prompt, etc)
#
# WHAT WE CANNOT TEST (requires interactive shell):
# ❌ Actual visual prompt repaint (requires terminal emulator)
# ❌ commandline -f repaint actually working (requires active prompt)
# ❌ SIGUSR1 signal delivery from external process
# ❌ End-to-end git → watcher → signal → repaint flow
#
# For manual testing of visual updates, use:
#   tests/fish/e2e_interactive_session.test.fish (a real pty session, #645)
#
# Run with: fish tests/fish/sigusr1_repaint.test.fish

# Source the IPC file to get the SIGUSR1 handlers. Use the repo copy, not the
# installed ~/.config/fish/gpy (which is absent on a clean CI runner and made
# this test silently pass only where a local gpy install happened to be
# autoloaded).
source fish/core/util.fish
source fish/core/ipc.fish

set -g test_count 0
set -g pass_count 0

function run_test
    set -g test_count (math $test_count + 1)
    if $argv
        set -g pass_count (math $pass_count + 1)
    end
end

# Test 1: Verify SIGUSR1 handler function exists
function test_sigusr1_handler_exists
    if functions -q __gpy_sigusr1_handler
        echo "PASS: __gpy_sigusr1_handler function exists"
        return 0
    else
        echo "FAIL: __gpy_sigusr1_handler function not found"
        return 1
    end
end

# Test 2: Verify variable change handler exists
function test_repaint_variable_handler_exists
    if functions -q __gpy_repaint_on_variable
        echo "PASS: __gpy_repaint_on_variable function exists"
        return 0
    else
        echo "FAIL: __gpy_repaint_on_variable function not found"
        return 1
    end
end

# Test 3: Verify SIGUSR1 handler is registered for the signal
function test_sigusr1_handler_registered
    set -l handlers (functions --handlers-type signal SIGUSR1 2>/dev/null)

    if string match -q "*__gpy_sigusr1_handler*" -- $handlers
        echo "PASS: __gpy_sigusr1_handler registered for SIGUSR1 signal"
        return 0
    else
        echo "FAIL: __gpy_sigusr1_handler not registered for SIGUSR1 signal"
        echo "  Registered handlers: $handlers"
        return 1
    end
end

# Test 4: Verify variable handler is registered for the trigger variable
function test_variable_handler_registered
    set -l handlers (functions --handlers-type variable __gpy_repaint_trigger 2>/dev/null)

    if string match -q "*__gpy_repaint_on_variable*" -- $handlers
        echo "PASS: __gpy_repaint_on_variable registered for __gpy_repaint_trigger variable"
        return 0
    else
        echo "FAIL: __gpy_repaint_on_variable not registered for variable changes"
        echo "  Registered handlers: $handlers"
        return 1
    end
end

# Test 5: Verify the variable-based repaint pattern works
function test_variable_repaint_pattern
    # Clear any existing trigger value
    set -e __gpy_repaint_trigger

    # Set a test flag to detect if the handler was called
    set -g __test_repaint_called 0

    # Temporarily override the repaint handler to set our test flag
    function __gpy_repaint_on_variable_test --on-variable __gpy_repaint_trigger
        set -g __test_repaint_called 1
    end

    # Trigger the pattern by setting the variable (simulates what SIGUSR1 handler does)
    set -g __gpy_repaint_trigger 1

    # Small delay to let event handlers fire
    sleep 0.05

    # Check if our test handler was called
    if test "$__test_repaint_called" = 1
        echo "PASS: Variable change triggers repaint handler"
        set -e __test_repaint_called
        functions -e __gpy_repaint_on_variable_test
        return 0
    else
        echo "FAIL: Variable change did not trigger repaint handler"
        set -e __test_repaint_called
        functions -e __gpy_repaint_on_variable_test
        return 1
    end
end

# Test 6: Verify SIGUSR1 handler increments the trigger variable
function test_sigusr1_increments_trigger
    set -e __gpy_repaint_trigger

    # Call handler - should set trigger to 1
    __gpy_sigusr1_handler
    sleep 0.01

    if test "$__gpy_repaint_trigger" = 1
        echo "PASS: SIGUSR1 handler sets trigger variable (first call)"
    else
        echo "FAIL: SIGUSR1 handler did not set trigger to 1, got: $__gpy_repaint_trigger"
        set -e __gpy_repaint_trigger
        return 1
    end

    # Call again - should increment to 2
    __gpy_sigusr1_handler
    sleep 0.01

    if test "$__gpy_repaint_trigger" = 2
        echo "PASS: SIGUSR1 handler increments trigger variable (second call)"
        set -e __gpy_repaint_trigger
        return 0
    else
        echo "FAIL: SIGUSR1 handler did not increment trigger to 2, got: $__gpy_repaint_trigger"
        set -e __gpy_repaint_trigger
        return 1
    end
end

# Test 8: Regression test - ensure we're NOT using the broken patterns
function test_no_broken_patterns
    set -l handler_body (functions __gpy_sigusr1_handler)

    # Check that we're NOT using emit fish_prompt directly
    if string match -q "*emit fish_prompt*" -- $handler_body
        echo "FAIL: SIGUSR1 handler uses 'emit fish_prompt' (doesn't work for idle prompts)"
        return 1
    end

    # Check that we're NOT using commandline -f repaint directly in signal handler
    if string match -q "*commandline -f repaint*" -- $handler_body
        echo "FAIL: SIGUSR1 handler uses 'commandline -f repaint' directly (doesn't work from signals)"
        return 1
    end

    # Check that we ARE using the variable trigger pattern
    if not string match -q "*__gpy_repaint_trigger*" -- $handler_body
        echo "FAIL: SIGUSR1 handler doesn't use __gpy_repaint_trigger variable pattern"
        return 1
    end

    echo "PASS: SIGUSR1 handler uses correct variable-based pattern (not broken patterns)"
    return 0
end

# Run all tests
echo "=== SIGUSR1 Repaint Mechanism Tests ==="
echo ""

run_test test_sigusr1_handler_exists
run_test test_repaint_variable_handler_exists
run_test test_sigusr1_handler_registered
run_test test_variable_handler_registered
run_test test_variable_repaint_pattern
run_test test_sigusr1_increments_trigger
run_test test_no_broken_patterns

echo ""
echo "=== Test Summary ==="
echo "Passed: $pass_count / $test_count"

if test $pass_count -eq $test_count
    echo "✅ All tests passed"
    exit 0
else
    echo "❌ Some tests failed"
    exit 1
end
