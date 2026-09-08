#!/usr/bin/env fish
# Regression test for #307: an agent that responds to IPC but speaks a
# different wire protocol (e.g. an old daemon left running across an
# upgrade) must be detected shell-side, not silently treated as usable.
#
# __gpy_check_protocol_version existed but was never called anywhere before
# this fix. This exercises it directly against a mock agent that returns a
# canned {"op":"status"} response, covering both the mismatch and match
# cases.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Protocol version mismatch detection (#307)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a mock agent"
end
if not command -q socat; and not command -q nc
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
end

set -l test_tmp_dir (mktemp -d)
set -l test_result PASS

function _protocol_mismatch_cleanup --argument-names listener_pid
    kill -9 $listener_pid 2>/dev/null
end

# --- Case 1: mismatched protocol_version must be detected ---

set -l sock1 $test_tmp_dir/gpy-protocol-mismatch.sock
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.recv(4096)
conn.sendall(b"{\"AgentStatus\":{\"version\":\"9.9.9\",\"protocol_version\":99,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
conn.close()
' $sock1 &
set -l listener1_pid $last_pid

for i in (seq 1 20)
    test -S $sock1; and break
    sleep 0.1
end

if not test -S $sock1
    _protocol_mismatch_cleanup $listener1_pid
    rm -rf $test_tmp_dir
    test_skip "could not create test Unix socket listener"
end

set -gx GPY_AGENT_SOCKET_PATH $sock1
set -gx GPY_AGENT_ENABLED 1

if __gpy_check_protocol_version
    print_test_result "mismatched protocol_version is detected" FAIL "expected nonzero return"
    set test_result FAIL
else
    print_test_result "mismatched protocol_version is detected" PASS
end

_protocol_mismatch_cleanup $listener1_pid

# --- Case 2: matching protocol_version must not be flagged ---

set -l sock2 $test_tmp_dir/gpy-protocol-match.sock
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.recv(4096)
conn.sendall(b"{\"AgentStatus\":{\"version\":\"0.1.0\",\"protocol_version\":1,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
conn.close()
' $sock2 &
set -l listener2_pid $last_pid

for i in (seq 1 20)
    test -S $sock2; and break
    sleep 0.1
end

if not test -S $sock2
    _protocol_mismatch_cleanup $listener2_pid
    rm -rf $test_tmp_dir
    test_skip "could not create second test Unix socket listener"
end

set -gx GPY_AGENT_SOCKET_PATH $sock2

if __gpy_check_protocol_version
    print_test_result "matching protocol_version is not flagged" PASS
else
    print_test_result "matching protocol_version is not flagged" FAIL "expected zero return"
    set test_result FAIL
end

_protocol_mismatch_cleanup $listener2_pid
functions -e _protocol_mismatch_cleanup
rm -rf $test_tmp_dir

print_test_footer "Protocol version mismatch detection" $test_result

if test "$test_result" = PASS
    exit 0
else
    exit 1
end
