#!/usr/bin/env fish
# Test: __gpy_request returns nonzero on failure

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

# Point at a nonexistent socket so IPC fails with no fallback
set -gx GPY_AGENT_SOCKET_PATH /tmp/gpy-nonexistent-test-socket-$fish_pid.sock
set -gx GPY_AGENT_ENABLED 1
# Disable agent binary so oneshot fallback also fails
set -gx GPY_AGENT_BINARY_PATH /nonexistent/gpy-agent-binary

set -l result (__gpy_request ping)
set -l exit_status $status

if test $exit_status -eq 0
    echo "FAIL: __gpy_request returned 0 when agent is unavailable"
    exit 1
end

if test -n "$result"
    echo "FAIL: __gpy_request output '$result' when agent unavailable"
    exit 1
end

echo "PASS: __gpy_request returns nonzero with empty output when agent is unavailable"
