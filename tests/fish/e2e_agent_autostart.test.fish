#!/usr/bin/env fish
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::create_dir)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Agent Start and Registration
# ============================================================================
#
# This test validates that the GPY agent can start and that Fish shells
# can successfully register with the running agent.
#
# NOTE: True "auto-start" (agent starting via background process from fish_prompt)
# cannot be reliably tested in an automated fashion because:
# 1. fish -c doesn't preserve background processes after script termination
# 2. Background jobs need an interactive parent shell to persist
# 3. The supervisor process is explicitly designed to be detached/daemonized
#
# This test instead validates:
# - Agent can be started manually
# - Fish shells can initialize GPY properly
# - Shells can register with a running agent
# - Registration mechanism works correctly
#
# Real interactive sessions (repaint with no keystroke, two shells on one
# agent, config reload on a Fish client) are driven on a pseudo-terminal by
# tests/fish/e2e_interactive_session.test.fish (#645).
#
# Test Flow:
# 1. Ensure no agent is currently running
# 2. Start the agent manually (simulating auto-start result)
# 3. Verify the agent is responsive
# 4. Start a Fish shell and verify it can register
#
# Expected Runtime: ~5 seconds
#
# What this test validates:
# ✅ Agent can be started successfully
# ✅ Agent becomes responsive after start
# ✅ Fish shell successfully registers with agent
# ✅ Registration mechanism works
#
# What this test CANNOT validate (requires manual testing):
# ❌ Agent auto-starts from fish_prompt background job
# ❌ Supervisor process manages agent lifecycle
# ❌ Agent persists across multiple shell sessions

source (dirname (status -f))/../lib/test_helpers.fish

# Test setup
function setup
    print_test_header "E2E Test: Agent Auto-Start"
    init_test_env
end

# Test teardown
function teardown
    cleanup_test_files
end

# Main test
function test_agent_autostart
    echo "Ensuring no agent is running..."
    stop_test_agent

    # Verify agent is actually stopped. Socket-scoped (not a bare `pgrep -x
    # gpy-agent`, which cannot distinguish this test's own agent from an
    # unrelated live daemon elsewhere on the machine, see #486).
    if not __gpy_agent_stopped
        print_test_result "Agent Stop" "FAIL" "Agent still running after stop"
        return 1
    end
    echo "✓ Agent stopped"

    echo ""
    echo "Starting agent manually (simulating auto-start)..."
    if not start_test_agent
        print_test_result "Agent Start" "FAIL" "Failed to start agent"
        return 1
    end
    echo "✓ Agent started"
    print_test_result "Agent Start" "PASS"

    # Verify agent is responsive
    echo ""
    echo "Verifying agent is responsive..."
    if not wait_for_agent 5
        print_test_result "Agent Responsive" "FAIL" "Agent started but not responding"
        return 1
    end

    print_test_result "Agent Responsive" "PASS"

    # Verify agent status
    echo ""
    echo "Testing agent ping..."
    set -l ping_result (gpy-agent status 2>&1)
    if not string match -q "*Running and Responding*" -- $ping_result
        print_test_result "Agent Ping" "FAIL" "Ping failed: $ping_result"
        return 1
    end
    print_test_result "Agent Ping" "PASS"

    # CI runners never run install-dev.fish, so ~/.config/fish/gpy does not
    # exist there (it only does on a contributor machine that has installed
    # GPY for normal use). Source the repo's own core/init.fish directly,
    # the same way fresh_install.test.fish and theme_prompt_render.test.fish
    # already do, so this test doesn't depend on out-of-band install state.
    set -l script_dir (dirname (status --filename))
    set -l repo_root (cd "$script_dir/../.." && pwd)

    # Test that a Fish shell can initialize GPY and register with the agent
    echo ""
    echo "Testing Fish shell registration..."
    set -l register_result (fish -c "
        source $repo_root/fish/core/init.fish >/dev/null 2>&1
        fish_prompt >/dev/null 2>&1
        if test \$status -eq 0
            echo REGISTERED
        end
    " 2>&1)

    if string match -q "*REGISTERED*" -- $register_result
        print_test_result "Shell Registration" "PASS"
    else
        print_test_result "Shell Registration" "FAIL" "Shell failed to register: $register_result"
        return 1
    end

    # Test that GPY initialization doesn't error out
    echo ""
    echo "Testing GPY initialization completes without errors..."
    set -l init_output (fish -c "
        source $repo_root/fish/core/init.fish
        echo \$status
    " 2>&1)
    set -l init_result $init_output[-1]

    if test "$init_result" = "0"
        print_test_result "GPY Initialization" "PASS"
    else
        set -l init_diagnostics (string join "; " -- $init_output[1..-2])
        print_test_result "GPY Initialization" "FAIL" "Init returned status: $init_result; output: $init_diagnostics"
        return 1
    end

    echo ""
    echo "========================================="
    echo "✅ E2E Agent Start & Registration Test: PASS"
    echo "========================================="
    return 0
end

# Run the test
setup
set -l test_result 0
test_agent_autostart
set test_result $status
teardown

exit $test_result
