#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test Infrastructure Helpers
#
# This file provides utility functions for end-to-end integration tests that
# validate the live update functionality of GPY.
#
# Key functions:
# - start_test_agent: Start agent in test mode with debug logging
# - stop_test_agent: Stop agent and cleanup
# - register_test_client: Spawn Fish subprocess that counts signals
# - count_signals: Count SIGURG doorbell rings received by test client
# - wait_for_agent / wait_for_signal / poll_until: bounded waits (never sleep
#   to wait for state; poll_until SECS CMD... is the general form)
# - cleanup_test_files: Remove temporary test files
# - test_guard_arm / test_track_pid / test_track_pidfile / test_track_dir /
#   test_track_socket / test_track_cleanup: teardown guard. Everything a test
#   registers is killed, removed or called back on every exit path (normal
#   exit, failed assertion, `exit N`, test_skip, SIGINT, SIGTERM, SIGHUP).
#   init_test_env arms it and registers its own sandbox and agent.

# Global test state
set -g __gpy_root (realpath (dirname (status -f))/../..)
set -g __gpy_test_tmp_dir /tmp/gpy-e2e-test-$fish_pid
set -g __gpy_test_debug_log $__gpy_test_tmp_dir/debug.log
set -g __gpy_test_signal_dir $__gpy_test_tmp_dir/signals
set -g __gpy_test_client_pids

# Teardown guard state. Only the fish process that armed the guard acts on it
# (see test_guard_arm), so a child `fish -c` that merely sources this file can
# never stop its parent's agent.
set -g __gpy_guard_owner ""
set -g __gpy_guard_pids
set -g __gpy_guard_dirs
set -g __gpy_guard_sockets
set -g __gpy_guard_pidfiles
set -g __gpy_guard_hooks
set -g __gpy_test_env_active 0

# Arm the teardown guard for this process. Idempotent.
#
# `--on-event fish_exit` covers every way this script ends on its own: end of
# file, `exit N`, a failed assertion, test_skip. A fatal signal never fires
# fish_exit, so SIGINT, SIGTERM and SIGHUP get handlers that tear down and then
# re-raise the signal (an `exit` inside a signal handler loses its status).
# fish runs a signal handler once the current foreground command returns, so
# an interactive Ctrl-C (delivered to the whole foreground group) tears down
# at once. SIGKILL cannot be caught: the next init_test_env sweeps what such a
# run left behind.
function test_guard_arm
    test "$__gpy_guard_owner" = "$fish_pid"; and return 0
    set -g __gpy_guard_owner $fish_pid
    function __gpy_guard_on_exit --on-event fish_exit
        __gpy_guard_teardown
    end
    function __gpy_guard_on_int --on-signal SIGINT
        __gpy_guard_die INT
    end
    function __gpy_guard_on_term --on-signal SIGTERM
        __gpy_guard_die TERM
    end
    function __gpy_guard_on_hup --on-signal SIGHUP
        __gpy_guard_die HUP
    end
end

# Tear down, then take the signal's default action by erasing the handlers and
# raising it again.
function __gpy_guard_die --argument-names signal
    __gpy_guard_teardown
    functions -e __gpy_guard_on_int __gpy_guard_on_term __gpy_guard_on_hup
    kill -$signal $fish_pid
end

# Register a background process (a stub listener, a test client, a daemon) the
# test started itself. Arms the guard.
function test_track_pid
    test_guard_arm
    set -a __gpy_guard_pids $argv
end

# Register a scratch directory. Teardown kills whatever holds a unix socket
# under it (an agent the test, or a shell under test, autostarted there) and
# then removes it. Arms the guard.
function test_track_dir
    test_guard_arm
    set -a __gpy_guard_dirs $argv
end

# Register a unix socket path an agent was started on; teardown kills its
# holder and removes the file. Arms the guard.
function test_track_socket
    test_guard_arm
    set -a __gpy_guard_sockets $argv
end

# Register a PID file (the supervisor's, say): whatever PID it names when
# teardown runs is killed. The PID is read at teardown, so a process started
# after registration is still covered. Arms the guard.
function test_track_pidfile
    test_guard_arm
    set -a __gpy_guard_pidfiles $argv
end

# Register a function to call during teardown, for cleanup the other
# registrations cannot express (it runs first, before PIDs are killed and
# directories removed). It must be idempotent and must not call `exit`. Arms
# the guard.
function test_track_cleanup
    test_guard_arm
    set -a __gpy_guard_hooks $argv
end

# TERM every PID, wait (bounded) for them to leave, then KILL the stragglers.
function __gpy_guard_kill_pids
    set -l pids
    for pid in $argv
        kill -0 $pid 2>/dev/null; and set -a pids $pid
    end
    set -q pids[1]; or return 0
    kill $pids 2>/dev/null
    poll_until 2 __gpy_pids_gone $pids
    kill -9 $pids 2>/dev/null
end

# Every unix socket file under $argv (recursive), one per line.
function __gpy_guard_sockets_under
    for dir in $argv
        test -d "$dir"; and find "$dir" -type s 2>/dev/null
    end
end

function __gpy_guard_teardown
    test "$__gpy_guard_owner" = "$fish_pid"; or return 0
    # Run once, even if cleanup_test_files or `exit` re-enters.
    set -g __gpy_guard_owner ""

    if test "$__gpy_test_env_active" = 1
        cleanup_test_files
    end
    for hook in $__gpy_guard_hooks
        $hook
    end

    set -l sockets $__gpy_guard_sockets (__gpy_guard_sockets_under $__gpy_guard_dirs)
    set -l file_pids
    for pidfile in $__gpy_guard_pidfiles
        test -s "$pidfile"; and set -a file_pids (string trim <$pidfile)
    end
    set -l holders
    for socket in $sockets
        set -a holders (__gpy_pids_for_socket $socket)
    end
    if set -q sockets[1]; and not command -q lsof
        echo "WARNING: lsof unavailable; cannot stop agents bound under the test sandbox" >&2
    end
    __gpy_guard_kill_pids $__gpy_guard_pids $file_pids $holders
    for socket in $sockets
        rm -f $socket 2>/dev/null
    end
    for dir in $__gpy_guard_dirs
        rm -rf $dir 2>/dev/null
    end
    set -g __gpy_guard_pids
    set -g __gpy_guard_dirs
    set -g __gpy_guard_sockets
    set -g __gpy_guard_pidfiles
end

# Resolve the PID(s) holding a Unix socket open. Prints one PID per line;
# prints nothing if the socket doesn't exist or lsof isn't available. This
# lets cleanup target the exact process that owns a specific socket instead
# of matching by process name, which cannot distinguish a test agent from a
# contributor's live dogfooding daemon.
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
    # repo instead of its throwaway one.
    set -e GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX GIT_NAMESPACE

    # Create temp directories
    rm -rf $__gpy_test_tmp_dir 2>/dev/null
    mkdir -p $__gpy_test_tmp_dir
    mkdir -p $__gpy_test_signal_dir
    set -g __gpy_test_env_active 1
    test_guard_arm

    # Isolate configuration, cache, and socket path. XDG_CACHE_HOME must be
    # isolated too: fish/core/init.fish's __gpy_load_theme fast path sources
    # whatever theme-export cache it finds there before ever consulting
    # XDG_CONFIG_HOME, so leaving it unset lets a live dogfooding daemon's
    # real cache leak into the test.
    set -gx XDG_CONFIG_HOME $__gpy_test_tmp_dir/config
    set -gx XDG_CACHE_HOME $__gpy_test_tmp_dir/cache
    set -gx GPY_AGENT_SOCKET_PATH $__gpy_test_tmp_dir/gpy.sock
    # The runtime root (shell PID files, supervisor.pid) then falls back to
    # the isolated $XDG_CACHE_HOME/gpy instead of the caller's runtime dir.
    set -e XDG_RUNTIME_DIR
    mkdir -p $XDG_CONFIG_HOME $XDG_CACHE_HOME

    # Test this checkout's gpy-agent, never an installed one: a standalone
    # `fish tests/fish/<file>` otherwise resolves whatever the caller's PATH
    # (or fish_user_paths) holds. Same rule as scripts/test_fish.sh and
    # tests/lib/shell_e2e.sh.
    set -l target_dir $__gpy_root/gpy-agent/target
    test -n "$CARGO_TARGET_DIR"; and set target_dir $CARGO_TARGET_DIR
    if not test -x $target_dir/debug/gpy-agent
        echo "Building debug gpy-agent for the Fish tests..."
        # Build from gpy-agent/ so rustup honours its rust-toolchain.toml pin.
        set -l prev_dir $PWD
        cd $__gpy_root/gpy-agent; or return 1
        cargo build --quiet --bin gpy --bin gpy-agent
        set -l build_status $status
        cd $prev_dir
        test $build_status -eq 0; or return 1
    end
    set -gx PATH $target_dir/debug $PATH

    # Belt-and-braces guard: refuse to proceed if the test socket doesn't look
    # like it lives under a throwaway temp dir. Everything below (leftover-
    # process cleanup, and stop_test_agent's force-kill) trusts that scoping
    # to decide what it's safe to kill; a path widened toward something shared
    # must fail loudly here rather than risk killing a live dogfooding daemon.
    if not string match -q "/tmp/*" -- $GPY_AGENT_SOCKET_PATH
        echo "ERROR: refusing to proceed — GPY_AGENT_SOCKET_PATH ($GPY_AGENT_SOCKET_PATH) is not under /tmp" >&2
        return 1
    end

    __gpy_sweep_stale_sandboxes
end

# Remove sandboxes a previous run could not tear down (SIGKILL, power loss):
# /tmp/gpy-e2e-test-<pid> directories whose owning PID is dead. A directory
# whose PID is alive belongs to a concurrent run and is left alone. Agents are
# resolved to a specific PID by the socket they hold (lsof), never matched by
# process name: pkill -x would also SIGKILL a contributor's live dogfooding
# daemon, and pkill -f would also match the `fish -c ...` test runner itself,
# whose command line embeds the literal string "gpy-agent" via the PATH
# override.
function __gpy_sweep_stale_sandboxes
    set -l leftover_pids
    set -l stale_dirs
    for dir in /tmp/gpy-e2e-test-*
        test -d "$dir"; or continue
        set -l owner (string replace '/tmp/gpy-e2e-test-' '' -- $dir)
        string match -qr '^[0-9]+$' -- $owner; or continue
        kill -0 $owner 2>/dev/null; and continue
        set -a stale_dirs $dir
        for socket in (__gpy_guard_sockets_under $dir)
            set -a leftover_pids (__gpy_pids_for_socket $socket)
        end
    end
    if set -q leftover_pids[1]
        kill -9 $leftover_pids 2>/dev/null
        poll_until 3 __gpy_pids_gone $leftover_pids
    end
    for dir in $stale_dirs
        rm -rf $dir 2>/dev/null
    end
end

# Predicate for poll_until: true when none of the given PIDs is still alive.
function __gpy_pids_gone
    for pid in $argv
        kill -0 $pid 2>/dev/null; and return 1
    end
    return 0
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
    # Try graceful shutdown first, then poll for the process to actually exit.
    # This helper runs on every start_test_agent and every restart, so the
    # common-case wait compounds across the whole suite.
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

    poll_until $timeout_seconds __gpy_agent_responding
end

# Predicate for poll_until: true when the agent reports "Running and Responding".
function __gpy_agent_responding
    gpy-agent status 2>&1 | grep -q "Running and Responding"
end

# Poll until a predicate succeeds, or a timeout elapses.
#
# Use instead of fixed `sleep N` padding before an assertion: wait only until
# the condition the test is about to assert actually holds. Typically resolves
# in ~100ms, while a genuinely failing case still exits at the timeout and lets
# the following assertion report the failure.
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
        touch $signal_dir/ready-$fish_pid

        # Wait for specified duration using a loop to remain responsive to signals
        set -l end_time (math (date +%s) + $duration)
        while test (date +%s) -lt $end_time
            sleep 0.5
        end
    ' -- $__gpy_test_signal_dir $__gpy_root $client_cwd $duration >/dev/null 2>&1 &

    set -l client_pid $last_pid
    set -a __gpy_test_client_pids $client_pid

    # Wait for the client to finish registering and its first prompt
    poll_until 5 test -e $__gpy_test_signal_dir/ready-$client_pid

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

# Predicate for poll_until: the client has received at least $argv[2] signals.
function __gpy_client_signals_at_least --argument-names client_pid minimum_count
    test (count_signals $client_pid) -ge $minimum_count
end

# Wait until a test client has received at least a number of SIGURG rings.
# Arguments: client PID, minimum count, timeout in seconds
# Returns: 0 once the count is reached, 1 on timeout
function wait_for_signal --argument-names client_pid minimum_count timeout_seconds
    poll_until $timeout_seconds __gpy_client_signals_at_least $client_pid $minimum_count
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
    set -g __gpy_test_env_active 0

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

# Skip contract: a test that cannot run because a prerequisite is missing
# calls `test_skip "reason"`. Locally that prints `SKIP: reason` and exits 0;
# under `CI` it exits 1, so the gate can never go green on a test that did
# not actually execute. Never `exit 0` on a missing prerequisite directly.
# The Bash/Zsh twin lives in tests/lib/shell_e2e.sh.
function test_skip
    echo "SKIP: $argv"
    if set -q CI; and test -n "$CI"
        echo "FAIL: a skipped test is a failure under CI"
        exit 1
    end
    exit 0
end

# Nanoseconds since the epoch, identical on every platform. `date
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

# Move a file's mtime (and atime) $argv[2] seconds into the past. Use it where
# a test needs "this file is older than that edit" across a coarse, one-second
# timestamp resolution: backdating is instant, a `sleep 1.1` is not.
function test_backdate --argument-names file seconds
    if test -z "$__gpy_test_clock_bin"
        test_skip "perl not installed, cannot backdate a file"
    end
    $__gpy_test_clock_bin -e 'my $t = time() - $ARGV[1]; utime($t, $t, $ARGV[0]) or die "utime: $!"' $file $seconds
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
