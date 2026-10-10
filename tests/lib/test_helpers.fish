#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test Infrastructure Helpers
# ============================================================================
#
# This file provides utility functions for end-to-end integration tests that
# validate the live update functionality of GPY.
#
# Key functions:
# - start_test_agent: Start agent in test mode with debug logging
# - stop_test_agent: Stop agent and cleanup
# - register_test_client: Spawn Fish subprocess that counts signals
# - count_signals: Count SIGURG doorbell rings received by test client
# - wait_for_agent: Wait for agent to become responsive
# - cleanup_test_files: Remove temporary test files

# Global test state
set -g __gpy_root (realpath (dirname (status -f))/../..)
set -g __gpy_test_tmp_dir /tmp/gpy-e2e-test-$fish_pid
set -g __gpy_test_debug_log $__gpy_test_tmp_dir/debug.log
set -g __gpy_test_signal_dir $__gpy_test_tmp_dir/signals
set -g __gpy_test_client_pids

# Resolve the PID(s) holding a Unix socket open. Prints one PID per line;
# prints nothing if the socket doesn't exist or lsof isn't available. This
# lets cleanup target the exact process that owns a specific socket instead
# of matching by process name, which cannot distinguish a test agent from a
# contributor's live dogfooding daemon (#484).
function __gpy_pids_for_socket --argument-names socket_path
    test -S "$socket_path"; or return 1
    command -q lsof; or return 1
    lsof -t "$socket_path" 2>/dev/null
end

# Force-kill whatever process (if any) is holding the given socket open, then
# remove the socket file. Never matches by process name — see
# __gpy_pids_for_socket. If lsof isn't available there is no safe way to
# identify the owning process, so this warns and leaves the socket alone
# rather than falling back to a name-based kill.
function __gpy_kill_socket_owner --argument-names socket_path
    if not command -q lsof
        if test -S "$socket_path"
            echo "WARNING: lsof unavailable; cannot safely target the process on $socket_path" >&2
        end
        return 1
    end

    for pid in (__gpy_pids_for_socket "$socket_path")
        kill -9 $pid 2>/dev/null
    end
    rm -f "$socket_path" 2>/dev/null
end

# Initialize test environment
function init_test_env
    # Drop any inherited repo-scoping git env (GIT_DIR/GIT_WORK_TREE/...). Left
    # set — e.g. when tests run from a git pre-push hook — they override the
    # `git -C <tmpdir>` used by fixtures, so a test's git ops would hit the real
    # repo instead of its throwaway one (#275).
    set -e GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX GIT_NAMESPACE

    # Create temp directories
    rm -rf $__gpy_test_tmp_dir 2>/dev/null
    mkdir -p $__gpy_test_tmp_dir
    mkdir -p $__gpy_test_signal_dir

    # Isolate configuration, cache, and socket path. XDG_CACHE_HOME must be
    # isolated too: fish/core/init.fish's __gpy_load_theme fast path sources
    # whatever theme-export cache it finds there before ever consulting
    # XDG_CONFIG_HOME, so leaving it unset lets a live dogfooding daemon's
    # real cache leak into the test (#402).
    set -gx XDG_CONFIG_HOME $__gpy_test_tmp_dir/config
    set -gx XDG_CACHE_HOME $__gpy_test_tmp_dir/cache
    set -gx GPY_AGENT_SOCKET_PATH $__gpy_test_tmp_dir/gpy.sock
    # The runtime root (shell PID files, supervisor.pid) then falls back to
    # the isolated $XDG_CACHE_HOME/gpy instead of the caller's runtime dir (#665).
    set -e XDG_RUNTIME_DIR
    mkdir -p $XDG_CONFIG_HOME $XDG_CACHE_HOME

    # Belt-and-braces guard: refuse to proceed if the test socket doesn't look
    # like it lives under a throwaway temp dir. Everything below (leftover-
    # process cleanup, and stop_test_agent's force-kill) trusts that scoping
    # to decide what it's safe to kill; a future refactor that widens this
    # path back toward something shared must fail loudly here instead of
    # silently reopening #484.
    if not string match -q "/tmp/*" -- $GPY_AGENT_SOCKET_PATH
        echo "ERROR: refusing to proceed — GPY_AGENT_SOCKET_PATH ($GPY_AGENT_SOCKET_PATH) is not under /tmp" >&2
        return 1
    end

    # Clean up any leftover test agents from a previous crashed run. Scoped to
    # this file's own /tmp/gpy-e2e-test-*/gpy.sock naming convention and
    # resolved to a specific PID via lsof, never matched by process name:
    # pkill -x would also SIGKILL a contributor's live dogfooding daemon, and
    # pkill -f would also match the `fish -c ...` test runner itself, whose
    # command line embeds the literal string "gpy-agent" via the PATH
    # override (#285).
    for leftover_socket in /tmp/gpy-e2e-test-*/gpy.sock
        __gpy_kill_socket_owner "$leftover_socket"
    end
    sleep 1
end

# Start the GPY agent in test mode
# Returns: 0 on success, 1 on failure
function start_test_agent
    # Set debug logging for this test
    set -gx GPY_DEBUG_LOG $__gpy_test_debug_log

    # Kill any existing agents
    stop_test_agent

    # Start the agent
    if not command -q gpy-agent
        echo "ERROR: gpy-agent not found in PATH" >&2
        return 1
    end

    gpy-agent start >$__gpy_test_tmp_dir/agent.log 2>&1
    if test $status -ne 0
        echo "ERROR: Failed to start gpy-agent" >&2
        return 1
    end

    # Wait for agent to become responsive
    if not wait_for_agent 5
        echo "ERROR: Agent did not become responsive" >&2
        return 1
    end

    return 0
end

# Stop the GPY agent and cleanup
function stop_test_agent
    # Try graceful shutdown first, then poll for the process to actually exit
    # instead of a fixed `sleep 1`. This helper runs on every start_test_agent
    # and every restart, so shaving the common-case wait compounds across the
    # whole suite.
    gpy-agent stop >/dev/null 2>&1
    poll_until 3 __gpy_agent_stopped

    # Force kill whatever still holds this test's own socket (see
    # __gpy_kill_socket_owner for why this is scoped by socket, not by
    # process name), then poll again for it to leave the process table.
    __gpy_kill_socket_owner "$GPY_AGENT_SOCKET_PATH"
    poll_until 3 __gpy_agent_stopped

    # Verify agent is stopped
    if not __gpy_agent_stopped
        echo "WARNING: Failed to stop agent cleanly" >&2
    end
end

# Predicate for poll_until: true when this test's own agent socket is gone.
# Scoped to $GPY_AGENT_SOCKET_PATH, not a bare process-name check: a
# `pgrep -x gpy-agent` here would wrongly report "not stopped" (and burn the
# full poll timeout) whenever a contributor's live dogfooding daemon happens
# to be running on the same machine, independent of this test's own agent.
function __gpy_agent_stopped
    not test -S "$GPY_AGENT_SOCKET_PATH"
end

# Predicate for poll_until: true when the agent reports >=1 registered client.
function __gpy_agent_has_registered_client
    gpy-agent status 2>/dev/null | grep -qE 'Registered Clients: [1-9]'
end

# Wait for agent to become responsive
# Arguments: $argv[1] - timeout in seconds (default: 5)
# Returns: 0 if agent responds, 1 on timeout
function wait_for_agent
    set -l timeout_seconds 5
    if set -q argv[1]
        set timeout_seconds $argv[1]
    end

    set -l attempts (math "$timeout_seconds * 2")
    for i in (seq 1 $attempts)
        if gpy-agent status 2>&1 | grep -q "Running and Responding"
            return 0
        end
        sleep 0.5
    end

    return 1
end

# Poll until a predicate succeeds, or a timeout elapses.
#
# Replaces fixed `sleep N` padding before an assertion: instead of waiting the
# worst-case duration every run, wait only until the condition the test is about
# to assert actually holds. Typically resolves in ~100ms rather than seconds,
# while a genuinely failing case still exits at the timeout and lets the
# following assertion report the failure exactly as a fixed sleep would.
#
# Arguments:
#   $argv[1]  - timeout in seconds (fractional allowed)
#   $argv[2..] - a command/function name plus its arguments; polled every 100ms
#                and considered satisfied when it exits 0.
# Returns: 0 as soon as the predicate succeeds, 1 if the timeout elapses.
function poll_until
    set -l timeout_seconds $argv[1]
    set -l predicate $argv[2..-1]
    set -l attempts (math "ceil($timeout_seconds / 0.1)")

    for i in (seq 1 $attempts)
        if $predicate
            return 0
        end
        sleep 0.1
    end

    return 1
end

# Register a test client that counts SIGURG doorbell rings
# Arguments:
#   $argv[1] - duration to run (seconds)
#   $argv[2] - working directory to register from (defaults to current directory)
# Returns: PID of the test client process
function register_test_client
    set -l duration 10
    if set -q argv[1]
        set duration $argv[1]
    end

    set -l client_cwd $PWD
    if set -q argv[2]
        set client_cwd $argv[2]
    end

    # Spawn a Fish subprocess that registers and counts signals. The paths and
    # duration travel as $argv, never interpolated into the code string (they
    # may contain spaces or quotes).
    fish -c '
        set -l signal_dir $argv[1]
        set -l repo_root $argv[2]
        set -l client_cwd $argv[3]
        set -l duration $argv[4]
        set -g signal_file $signal_dir/client-$fish_pid

        # Define our test signal handler BEFORE GPY init
        function __test_count_signal --on-signal SIGURG
            echo SIGNAL >> $signal_file
        end

        # Source GPY initialization from repo
        source $repo_root/fish/core/init.fish >/dev/null 2>&1

        # Source GPY functions
        for f in $repo_root/fish/functions/*.fish
            source $f
        end

        cd $client_cwd

        # Trigger registration
        __gpy_register_with_agent >/dev/null 2>&1

        # Trigger fish_prompt to ensure state is initialized
        fish_prompt >/dev/null 2>&1

        # Wait for specified duration using a loop to remain responsive to signals
        set -l end_time (math (date +%s) + $duration)
        while test (date +%s) -lt $end_time
            sleep 0.5
        end
    ' -- $__gpy_test_signal_dir $__gpy_root $client_cwd $duration >/dev/null 2>&1 &

    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    # Give the client time to register
    sleep 1

    echo $client_pid
end

# Count signals received by a test client
# Arguments: $argv[1] - client PID
# Returns: Number of signals received
function count_signals
    set -l client_pid $argv[1]
    set -l signal_file $__gpy_test_signal_dir/client-$client_pid

    if not test -f $signal_file
        echo 0
        return
    end

    wc -l <$signal_file | string trim
end

# Trigger a git change to activate file watcher
# Arguments: $argv[1] - repository path (default: PWD)
# Returns: 0 on success
function trigger_git_change
    set -l repo_path $PWD
    if set -q argv[1]
        set repo_path $argv[1]
    end

    # Create and stage a test file
    set -l test_file $repo_path/.gpy-test-$fish_pid
    touch $test_file
    git -C $repo_path add $test_file >/dev/null 2>&1

    return $status
end

# Clean up git test files
# Arguments: $argv[1] - repository path (default: PWD)
function cleanup_git_test_files
    set -l repo_path $PWD
    if set -q argv[1]
        set repo_path $argv[1]
    end

    # Use find to avoid wildcard expansion errors when no files exist
    set -l test_files (find $repo_path -maxdepth 1 -name '.gpy-test-*' 2>/dev/null)
    if test -n "$test_files"
        git -C $repo_path reset HEAD $test_files >/dev/null 2>&1
        rm -f $test_files 2>/dev/null
    end
end

# Clean up all test files and processes
function cleanup_test_files
    # Kill all test client processes
    for pid in $__gpy_test_client_pids
        kill -9 $pid 2>/dev/null
    end
    set -e __gpy_test_client_pids

    # Stop the agent
    stop_test_agent

    # Remove temp directory
    rm -rf $__gpy_test_tmp_dir 2>/dev/null

    # Clean up git test files in current repo
    cleanup_git_test_files
end

# Wait for next minute boundary (:00 seconds)
# Returns: Number of seconds until next boundary
function wait_for_minute_boundary
    set -l current_second (date +%S)
    set -l seconds_to_wait (math "60 - $current_second + 2")

    echo "Waiting $seconds_to_wait seconds for next :00 boundary..." >&2
    sleep $seconds_to_wait

    return 0
end

# Get current second of minute
function get_current_second
    date +%S | string replace -r '^0' ''
end

# Create a temporary git repository for testing
# Returns: Path to the temporary repo
function create_test_repo
    set -l test_repo $__gpy_test_tmp_dir/test-repo
    rm -rf $test_repo 2>/dev/null
    mkdir -p $test_repo

    # Initialize git repo
    git -C $test_repo init >/dev/null 2>&1
    git -C $test_repo config user.email "test@gpy.test" 2>/dev/null
    git -C $test_repo config user.name "GPY Test" 2>/dev/null

    # Create initial commit
    echo test >$test_repo/README.md
    git -C $test_repo add README.md >/dev/null 2>&1
    git -C $test_repo commit -m "Initial commit" >/dev/null 2>&1

    echo $test_repo
end

# Skip contract (#650): a test that cannot run because a prerequisite is
# missing calls `test_skip "reason"`. Locally that prints `SKIP: reason` and
# exits 0; under `CI` it exits 1, so the gate can never go green on a test
# that did not actually execute. Never `exit 0` on a missing prerequisite
# directly. The Bash/Zsh twin lives in tests/lib/shell_e2e.sh.
function test_skip
    echo "SKIP: $argv"
    if set -q CI; and test -n "$CI"
        echo "FAIL: a skipped test is a failure under CI (#650)"
        exit 1
    end
    exit 0
end

# Nanoseconds since the epoch, identical on every platform (#848). `date
# +%s%N` is GNU-only: BSD/macOS date prints a literal `N`, which `math` then
# rejects. perl's Time::HiRes ships with macOS and every Linux base image.
# The interpreter is resolved once, at source time, so a test that later
# restricts PATH to force a fallback path can still time itself.
set -g __gpy_test_clock_bin (command -v perl)
function test_now_ns
    if test -z "$__gpy_test_clock_bin"
        test_skip "perl not installed, cannot take a nanosecond timestamp"
    end
    $__gpy_test_clock_bin -MTime::HiRes=time -e 'printf "%d\n", time() * 1e9'
end

# Print test header
function print_test_header
    echo ""
    echo "========================================"
    echo "$argv[1]"
    echo "========================================"
    echo ""
end

# Print test result
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

# Assert that a value equals expected
function assert_equals
    set -l actual $argv[1]
    set -l expected $argv[2]
    set -l message $argv[3]

    if test "$actual" = "$expected"
        return 0
    else
        echo "FAIL: $message" >&2
        echo "  Expected: $expected" >&2
        echo "  Actual:   $actual" >&2
        return 1
    end
end

# Assert that a value is greater than threshold
function assert_greater_than
    set -l actual $argv[1]
    set -l threshold $argv[2]
    set -l message $argv[3]

    if test $actual -gt $threshold
        return 0
    else
        echo "FAIL: $message" >&2
        echo "  Expected: > $threshold" >&2
        echo "  Actual:   $actual" >&2
        return 1
    end
end

# Assert that a value is less than threshold
function assert_less_than
    set -l actual $argv[1]
    set -l threshold $argv[2]
    set -l message $argv[3]

    if test $actual -lt $threshold
        return 0
    else
        echo "FAIL: $message" >&2
        echo "  Expected: < $threshold" >&2
        echo "  Actual:   $actual" >&2
        return 1
    end
end

# Print test footer
function print_test_footer
    set -l test_name $argv[1]
    set -l test_status $argv[2]

    if test "$test_status" = PASS
        echo "✅ $test_name: All tests passed"
    else
        echo "❌ $test_name: Tests failed"
    end
end

# Remove a client PID from the cleanup list
function remove_client_from_cleanup --argument-names target_pid
    set -l remaining
    for pid in $__gpy_test_client_pids
        if test $pid -ne $target_pid
            set -a remaining $pid
        end
    end
    if set -q remaining[1]
        set __gpy_test_client_pids $remaining
    else
        set -e __gpy_test_client_pids
    end
end
