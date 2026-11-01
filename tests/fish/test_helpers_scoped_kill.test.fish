#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# Scoped agent cleanup
#
# tests/lib/test_helpers.fish must clean up test agents by the socket they
# hold, never with `pkill -9 -x gpy-agent` / `pkill -x gpy-agent`: matching on
# process name alone cannot distinguish a test's own agent from any other
# gpy-agent running on the machine -- including a contributor's live
# dogfooding daemon.
#
# This test simulates that daemon with a decoy gpy-agent bound to its own,
# distinct socket path (never under test_helpers.fish's
# /tmp/gpy-e2e-test-*/gpy.sock naming convention), then drives the helper's
# cleanup path from its own fish subprocess and asserts the decoy is
# untouched throughout.
#
# A name-based kill fails this test: the decoy is a real `gpy-agent` process
# by name, so it would be killed regardless of which socket it holds.

set -g __gpy_root (realpath (dirname (status -f))/../..)
source $__gpy_root/tests/lib/test_helpers.fish
set -g __decoy_dir /tmp/gpy-484-decoy-$fish_pid
set -g __decoy_socket $__decoy_dir/decoy.sock
set -g __decoy_pid

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
        if gpy-agent status >/dev/null 2>&1
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
    gpy-agent status >/dev/null 2>&1
end

function stop_decoy_agent
    if test -n "$__decoy_pid"
        set -lx GPY_AGENT_SOCKET_PATH $__decoy_socket
        gpy-agent stop >/dev/null 2>&1
        # Wait (bounded) for the graceful stop before the force-kill below.
        for i in (seq 30)
            kill -0 $__decoy_pid 2>/dev/null; or break
            sleep 0.1
        end
        kill -9 $__decoy_pid 2>/dev/null
    end
    rm -rf $__decoy_dir 2>/dev/null
    set -e __decoy_pid
end

test_track_cleanup stop_decoy_agent

# Assert the decoy is still alive and responding on its own socket. Prints a
# FAIL result (with reason) and returns 1 if not.
function assert_decoy_survived
    set -l label $argv[1]

    if not decoy_alive
        fail "$label" "decoy PID $__decoy_pid is gone from the process table"
        return 1
    end

    if not decoy_responding
        fail "$label" "decoy PID $__decoy_pid is alive but not responding on $__decoy_socket"
        return 1
    end

    pass "$label"
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
        fail "test_helpers.fish cleanup ran cleanly" "subprocess exited $helper_status: $out"
        return 1
    end

    assert_decoy_survived "decoy survives test_helpers.fish init_test_env + stop_test_agent"
end

print_test_header "Scoped-kill regression test"

if not start_decoy_agent
    echo "ERROR: failed to start decoy gpy-agent for the test" >&2
    stop_decoy_agent
    exit 1
end
echo "Decoy gpy-agent started: PID $__decoy_pid, socket $__decoy_socket"

test_test_helpers_scoped_kill

stop_decoy_agent

test_finish "Scoped-kill regression test"
