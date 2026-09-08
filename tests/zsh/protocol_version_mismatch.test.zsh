#!/usr/bin/env zsh
# tests/zsh/protocol_version_mismatch.test.zsh
#
# Regression test for #307: an agent that responds to IPC but speaks a
# different wire protocol (e.g. an old daemon left running across an
# upgrade) must be detected shell-side, not silently treated as usable.
#
# Zsh had no protocol check at all before this fix; `gpy-agent status`
# exits 0 for any responsive agent regardless of protocol version, which
# __gpy_start_agent used as its sole "already running" gate. This exercises
# the new __gpy_check_protocol_version directly.

ROOT="$(cd "$(dirname "${(%):-%x}")/../.." && pwd)"
# Shared skip contract (#650): exits 0 locally, fails under CI.
emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a mock agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
test_result=0

cleanup() {
    kill -9 "$1" 2>/dev/null
}

# --- Case 1: mismatched protocol_version must be detected ---

sock1="$test_tmp_dir/gpy-protocol-mismatch.sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.recv(4096)
conn.sendall(b"{\"AgentStatus\":{\"version\":\"9.9.9\",\"protocol_version\":99,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
conn.close()
' "$sock1" &
listener1_pid=$!
disown "$listener1_pid" 2>/dev/null

for _ in $(seq 1 20); do
    [[ -S "$sock1" ]] && break
    sleep 0.1
done

if [[ ! -S "$sock1" ]]; then
    cleanup "$listener1_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock1"

if __gpy_check_protocol_version; then
    echo "FAIL: mismatched protocol_version is detected: expected nonzero return"
    test_result=1
else
    echo "PASS: mismatched protocol_version is detected"
fi

cleanup "$listener1_pid"

# --- Case 2: matching protocol_version must not be flagged ---

sock2="$test_tmp_dir/gpy-protocol-match.sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.recv(4096)
conn.sendall(b"{\"AgentStatus\":{\"version\":\"0.1.0\",\"protocol_version\":1,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
conn.close()
' "$sock2" &
listener2_pid=$!
disown "$listener2_pid" 2>/dev/null

for _ in $(seq 1 20); do
    [[ -S "$sock2" ]] && break
    sleep 0.1
done

if [[ ! -S "$sock2" ]]; then
    cleanup "$listener2_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create second test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock2"

if __gpy_check_protocol_version; then
    echo "PASS: matching protocol_version is not flagged"
else
    echo "FAIL: matching protocol_version is not flagged: expected zero return"
    test_result=1
fi

cleanup "$listener2_pid"
rm -rf "$test_tmp_dir"

exit "$test_result"
