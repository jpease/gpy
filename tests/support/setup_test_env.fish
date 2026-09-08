#!/usr/bin/env fish
# GPY Test Environment Setup
# Provides common utilities and configuration for GPY tests

# Test environment configuration
set -gx GPY_TEST_MODE 1
set -gx GPY_DEBUG 0 # Disable debug output during tests
set -gx GPY_FALLBACK_ENABLED 1 # Enable fallback for agent unavailable scenarios

# Isolate the theme-export cache dir. fish/core/init.fish's __gpy_load_theme
# fast path sources whatever cache it finds here before consulting any
# per-test XDG_CONFIG_HOME override, so a live dogfooding daemon's real cache
# can otherwise leak into the test (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

# Override agent paths for testing
function get_agent_binary_path
    # First check if we have a built agent
    set agent_candidates \
        "$PWD/gpy-agent/target/release/gpy-agent" \
        "$PWD/gpy-agent/target/debug/gpy-agent" \
        "$HOME/.config/fish/gpy/bin/gpy-agent" \
        (which gpy-agent 2>/dev/null)

    for candidate in $agent_candidates
        if test -f "$candidate" -a -x "$candidate"
            echo $candidate
            return 0
        end
    end

    echo gpy-agent # fallback
    return 1
end

# Single GPY-owned root all of this harness's test sockets live beneath.
# Honors $TMPDIR the same way the Rust test fixtures do (see
# gpy-agent/tests/common/fixtures.rs's gpy_test_root), so cleanup tooling
# (scripts/cleanup-test-agents.sh) can delete leaked sockets by location
# instead of matching by name alone (#619).
function __gpy_test_root
    set -l base $TMPDIR
    if test -z "$base"
        set base /tmp
    end
    # Trim a trailing slash -- TMPDIR often has one (e.g. macOS's
    # /var/folders/.../T/) and we join our own separator below.
    set base (string trim --right --chars=/ -- $base)

    set -l user $USER
    if test -z "$user"
        set user (id -un)
    end

    echo "$base/gpy-test-$user"
end

# Override socket path for testing to avoid conflicts
function gpy_agent_socket_path
    set test_runtime_dir (__gpy_test_root)
    mkdir -p $test_runtime_dir
    echo "$test_runtime_dir/gpy-test.sock"
end

# Test-specific agent configuration
set -gx GPY_AGENT_SOCKET_PATH (gpy_agent_socket_path)
set -gx GPY_AGENT_BINARY_PATH (get_agent_binary_path)

# Disable live updates for testing stability
set -gx GPY_LIVE_UPDATES_ENABLED 0
set -gx GPY_LIVE_DEBOUNCE_MS 0

# Test timeout configuration
set -gx GPY_IPC_TIMEOUT_MS 5000 # Generous timeout for CI environments
set -gx GPY_AGENT_STARTUP_TIMEOUT 10

# Override git check interval for faster testing
set -gx __git_check_interval 0

# Test logging
function test_log
    if test "$GPY_DEBUG" = 1
        echo "[TEST] $argv" >&2
    end
end

# Test assertion helpers
function assert_command_succeeds
    set cmd $argv[1]
    set description $argv[2..-1]

    test_log "Running: $cmd"
    eval $cmd
    set exit_code $status

    if test $exit_code -ne 0
        echo "FAIL: Command failed ($exit_code): $cmd" >&2
        if test (count $description) -gt 0
            echo "Description: $description" >&2
        end
        return 1
    end

    return 0
end

function assert_output_contains
    set expected $argv[1]
    set actual $argv[2]

    if not echo "$actual" | grep -q "$expected"
        echo "FAIL: Output does not contain '$expected'" >&2
        echo "Actual output: $actual" >&2
        return 1
    end

    return 0
end

function assert_file_exists
    set file_path $argv[1]

    if not test -f "$file_path"
        echo "FAIL: File does not exist: $file_path" >&2
        return 1
    end

    return 0
end

function assert_equals
    set expected $argv[1]
    set actual $argv[2]
    set description $argv[3..-1]

    if test "$expected" != "$actual"
        echo "FAIL: Values not equal" >&2
        echo "Expected: $expected" >&2
        echo "Actual:   $actual" >&2
        if test (count $description) -gt 0
            echo "Description: $description" >&2
        end
        return 1
    end

    return 0
end

function assert_performance_under
    set max_ms $argv[1]
    set actual_ms $argv[2]
    set operation $argv[3..-1]

    if test $actual_ms -gt $max_ms
        echo "FAIL: Performance budget exceeded" >&2
        echo "Budget: $max_ms ms" >&2
        echo "Actual: $actual_ms ms" >&2
        if test (count $operation) -gt 0
            echo "Operation: $operation" >&2
        end
        return 1
    end

    test_log "Performance OK: $actual_ms ms (budget: $max_ms ms) - $operation"
    return 0
end

# Test repository creation helpers
function create_test_git_repo
    set repo_path (mktemp -d)
    cd $repo_path

    git init -q
    git config user.email "gpy-test@example.com"
    git config user.name "GPY Test Suite"

    echo "# Test Repository" >README.md
    echo "Created by GPY test suite" >>README.md
    git add README.md
    git commit -q -m "Initial test commit"

    echo $repo_path
end

function create_test_language_repo
    set lang $argv[1]
    set repo_path (create_test_git_repo)
    cd $repo_path

    switch $lang
        case rust
            echo '[package]' >Cargo.toml
            echo 'name = "test-project"' >>Cargo.toml
            echo 'version = "0.1.0"' >>Cargo.toml

        case node nodejs javascript
            echo '{"name": "test-project", "version": "1.0.0"}' >package.json
            echo 'console.log("hello");' >index.js

        case python py
            echo 'def main(): pass' >main.py
            echo '[build-system]' >pyproject.toml
            echo 'requires = ["setuptools"]' >>pyproject.toml

        case go golang
            echo 'module test-project' >go.mod
            echo 'go 1.21' >>go.mod
            echo 'package main' >main.go

        case java
            mkdir -p src/main/java
            echo '<project><modelVersion>4.0.0</modelVersion><groupId>test</groupId><artifactId>test</artifactId><version>1.0.0</version></project>' >pom.xml
            echo 'public class Main { public static void main(String[] args) {} }' >src/main/java/Main.java

        case '*'
            echo "Unknown language: $lang" >&2
            rm -rf $repo_path
            return 1
    end

    git add .
    git commit -q -m "Add $lang project files"

    echo $repo_path
end

# Agent management helpers
function ensure_agent_stopped
    set test_socket (gpy_agent_socket_path)

    # Kill only the process holding this test suite's own socket open, never
    # by process name: pkill -x would also SIGKILL a contributor's live
    # dogfooding daemon, and pkill -f would also match the `fish -c ...` test
    # runner itself, whose command line embeds the literal string "gpy-agent"
    # via the PATH override (#285).
    if command -q lsof
        for pid in (lsof -t "$test_socket" 2>/dev/null)
            kill -9 $pid 2>/dev/null
        end
    else if test -S "$test_socket"
        echo "WARNING: lsof unavailable; cannot safely target the process on $test_socket" >&2
    end

    sleep 0.5

    # Clean up test socket files
    rm -f "$test_socket" "$test_socket.pid"
end

function wait_for_agent_ready
    set timeout $argv[1]
    test -n "$timeout"; or set timeout 5

    set deadline (math (date +%s) + $timeout)

    while test (date +%s) -lt $deadline
        if gpy_agent_ping >/dev/null 2>&1
            test_log "Agent ready"
            return 0
        end
        sleep 0.1
    end

    test_log "Agent not ready after $timeout seconds"
    return 1
end

# Performance timing helpers
function time_command_ms
    set start_time (date +%s%3N)
    eval $argv
    set exit_code $status
    set end_time (date +%s%3N)
    set elapsed (math $end_time - $start_time)

    # Return elapsed time in milliseconds via stdout
    echo $elapsed
    return $exit_code
end

# Mock external commands for testing
function mock_git_command
    # This would allow us to mock git responses for testing
    # without requiring actual git repositories
    echo "mock git implementation would go here"
end

# Test cleanup
function cleanup_test_environment
    ensure_agent_stopped

    # Clean up any temporary files -- scoped to this harness's own GPY-owned
    # root, never a wider /tmp/gpy-test-* glob that could remove an unrelated
    # same-named path (#619).
    rm -rf (__gpy_test_root) 2>/dev/null || true

    # Reset environment variables
    set -e GPY_TEST_MODE
end

# Setup signal handler for test cleanup
function __test_cleanup_handler --on-event fish_exit
    cleanup_test_environment
end

test_log "GPY test environment loaded"

function gpy_agent_ping
    __gpy_request ping
end
