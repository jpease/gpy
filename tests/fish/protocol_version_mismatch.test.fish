#!/usr/bin/env fish
# An agent that responds to IPC but speaks a different wire protocol (e.g. an
# old daemon left running across an upgrade) must be detected shell-side, not
# silently treated as usable.
#
# This exercises __gpy_check_protocol_version directly against a mock agent
# that returns a canned {"op":"status"} response, covering both the mismatch
# and match cases.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Protocol version mismatch detection"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a mock agent"
end
if not command -q socat; and not command -q nc
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
end

set -l test_tmp_dir (mktemp -d)
test_track_dir $test_tmp_dir

# --- Case 1: mismatched protocol_version must be detected ---

set -l sock1 $test_tmp_dir/gpy-protocol-mismatch.sock
test_stub_listener $sock1 reply \
    --reply '{"AgentStatus":{"version":"9.9.9","protocol_version":99,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'

set -gx GPY_AGENT_SOCKET_PATH $sock1
set -gx GPY_AGENT_ENABLED 1

if __gpy_check_protocol_version
    fail "mismatched protocol_version is detected" "expected nonzero return"
else
    pass "mismatched protocol_version is detected"
end

test_stop_pids $test_stub_pid

# --- Case 2: matching protocol_version must not be flagged ---

set -l sock2 $test_tmp_dir/gpy-protocol-match.sock
test_stub_listener $sock2 reply \
    --reply '{"AgentStatus":{"version":"0.1.0","protocol_version":2,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'

set -gx GPY_AGENT_SOCKET_PATH $sock2

if __gpy_check_protocol_version
    pass "matching protocol_version is not flagged"
else
    fail "matching protocol_version is not flagged" "expected zero return"
end

test_stop_pids $test_stub_pid

test_finish "Protocol version mismatch detection"
