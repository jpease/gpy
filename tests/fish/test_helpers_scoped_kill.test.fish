#!/usr/bin/env fish
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# Regression test: scoped agent cleanup (#484)
# ============================================================================
#
# tests/lib/test_helpers.fish and tests/support/setup_test_env.fish used to
# clean up test agents with `pkill -9 -x gpy-agent` / `pkill -x gpy-agent`,
# matched on process name alone. That cannot distinguish a test's own agent
# from any other gpy-agent running on the machine -- including a
# contributor's live dogfooding daemon.
#
# This test simulates that daemon with a decoy gpy-agent bound to its own,
# distinct socket path (never under test_helpers.fish's
# /tmp/gpy-e2e-test-*/gpy.sock naming convention, and never
# setup_test_env.fish's fixed /tmp/gpy-test-$USER/gpy-test.sock), then drives
# each helper file's cleanup path from its own fish subprocess and asserts
# the decoy is untouched throughout.
#
# Each helper file gets its own subprocess rather than being sourced
# together into one, because both files define same-named helper functions
# (e.g. assert_equals) with different signatures -- sourcing both into one
# process would let the second definition silently shadow the first.
#
# Without the fix (a raw `pkill -x gpy-agent` / `pkill -9 -x gpy-agent`) this
# test fails: the decoy is a real `gpy-agent` process by name, so it gets
# killed regardless of which socket it holds.

set -g __gpy_root (realpath (dirname (status -f))/../..)
set -g __decoy_dir /tmp/gpy-484-decoy-$fish_pid
set -g __decoy_socket $__decoy_dir/decoy.sock
set -g __decoy_pid

function print_test_header
    echo ""
    echo "========================================"
    echo "$argv[1]"
    echo "========================================"
    echo ""
end

function print_test_result
    set -l test_name $argv[1]
    set -l result $argv[2]

    if test "$result" = PASS
        echo "✅ $test_name: PASS"
        return 0
    else
        echo "❌ $test_name: FAIL"
        if set -q argv[3]
            echo "   Reason: $argv[3]"
        end
        return 1
    end
end

# Start a real gpy-agent bound to its own isolated socket, standing in for a
# contributor's live dogfooding daemon. Sets __decoy_pid on success.
function start_decoy_agent
    rm -rf $__decoy_dir 2>/dev/null
    mkdir -p $__decoy_dir

    set -lx GPY_AGENT_SOCKET_PATH $__decoy_socket
    set -lx XDG_CONFIG_HOME $__decoy_dir/config
    set -lx XDG_CACHE_HOME $__decoy_dir/cache
    mkdir -p $XDG_CONFIG_HOME $XDG_CACHE_HOME

    gpy-agent start >$__decoy_dir/agent.log 2>&1
    if test $status -ne 0
        cat $__decoy_dir/agent.log >&2
        return 1
    end

    for i in (seq 1 20)
        if gpy-agent status 2>&1 | grep -q "Running and Responding"
            set -g __decoy_pid (lsof -t "$__decoy_socket" 2>/dev/null | head -n 1)
            test -n "$__decoy_pid"; and return 0
        end
        sleep 0.25
    end

    return 1
end

function decoy_alive
    test -n "$__decoy_pid"; and kill -0 $__decoy_pid 2>/dev/null
end

function decoy_responding
    set -lx GPY_AGENT_SOCKET_PATH $__decoy_socket
    gpy-agent status 2>&1 | grep -q "Running and Responding"
end

function stop_decoy_agent
    if test -n "$__decoy_pid"
        set -lx GPY_AGENT_SOCKET_PATH $__decoy_socket
        gpy-agent stop >/dev/null 2>&1
        sleep 0.3
        kill -9 $__decoy_pid 2>/dev/null
    end
    rm -rf $__decoy_dir 2>/dev/null
    set -e __decoy_pid
end

# Assert the decoy is still alive and responding on its own socket. Prints a
# FAIL result (with reason) and returns 1 if not.
function assert_decoy_survived
    set -l label $argv[1]

    if not decoy_alive
        print_test_result $label FAIL "decoy PID $__decoy_pid is gone from the process table"
        return 1
    end

    if not decoy_responding
        print_test_result $label FAIL "decoy PID $__decoy_pid is alive but not responding on $__decoy_socket"
        return 1
    end

    print_test_result $label PASS
    return 0
end

function test_test_helpers_scoped_kill
    echo "Driving tests/lib/test_helpers.fish's init_test_env + stop_test_agent from a subprocess..."

    set -l out (fish -c '
        source $argv[1]/tests/lib/test_helpers.fish
        init_test_env; or exit 1
        start_test_agent >/dev/null 2>&1; or exit 1
        stop_test_agent
        exit 0
    ' -- $__gpy_root 2>&1)
    set -l helper_status $status

    if test $helper_status -ne 0
        print_test_result "test_helpers.fish cleanup ran cleanly" FAIL "subprocess exited $helper_status: $out"
        return 1
    end

    assert_decoy_survived "decoy survives test_helpers.fish init_test_env + stop_test_agent"
end

function test_setup_test_env_scoped_kill
    echo "Driving tests/support/setup_test_env.fish's ensure_agent_stopped from a subprocess..."

    set -l out (fish -c '
        source $argv[1]/tests/support/setup_test_env.fish >/dev/null 2>&1
        ensure_agent_stopped
        exit 0
    ' -- $__gpy_root 2>&1)
    set -l helper_status $status

    if test $helper_status -ne 0
        print_test_result "setup_test_env.fish cleanup ran cleanly" FAIL "subprocess exited $helper_status: $out"
        return 1
    end

    assert_decoy_survived "decoy survives setup_test_env.fish ensure_agent_stopped"
end

print_test_header "Scoped-kill regression test (#484)"

if not start_decoy_agent
    echo "ERROR: failed to start decoy gpy-agent for the test" >&2
    stop_decoy_agent
    exit 1
end
echo "Decoy gpy-agent started: PID $__decoy_pid, socket $__decoy_socket"

set -l overall_result 0

test_test_helpers_scoped_kill
or set overall_result 1

if not decoy_alive
    echo "Decoy died after the first subtest; restarting it for the second subtest..."
    start_decoy_agent; or set overall_result 1
end

test_setup_test_env_scoped_kill
or set overall_result 1

stop_decoy_agent

echo ""
if test $overall_result -eq 0
    echo "✅ Scoped-kill regression test (#484): PASS"
else
    echo "❌ Scoped-kill regression test (#484): FAIL"
end

exit $overall_result
