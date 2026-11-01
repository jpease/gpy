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
# No agent binary on PATH and none under a sandbox HOME, so the oneshot
# fallback also fails
set -gx HOME (mktemp -d)
set -l test_path
for dir in $PATH
    test -x "$dir/gpy-agent"; or set -a test_path $dir
end
set -gx PATH $test_path

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
