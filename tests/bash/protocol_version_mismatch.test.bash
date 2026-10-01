#!/usr/bin/env bash
# tests/bash/protocol_version_mismatch.test.bash
#
# Regression test for #307: an agent that responds to IPC but speaks a
# different wire protocol (e.g. an old daemon left running across an
# upgrade) must be detected shell-side, not silently treated as usable.
#
# Bash had no protocol check at all before this fix; the supervisor's
# __gpy_supervisor_is_running only pinged. This exercises the new
# __gpy_check_protocol_version directly, and confirms it now gates
# __gpy_supervisor_is_running so a responsive-but-mismatched agent is
# treated as not running.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract (#650): exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source bash/core/ipc.bash
source bash/core/supervisor.bash

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
conn.sendall(b"{\"AgentStatus\":{\"version\":\"0.1.0\",\"protocol_version\":2,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
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

# --- Case 3: __gpy_supervisor_is_running treats a mismatch as not running ---

sock3="$test_tmp_dir/gpy-protocol-supervisor.sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(2)
for _ in range(2):
    conn, _ = s.accept()
    req = conn.recv(4096)
    if b"ping" in req:
        conn.sendall(b"{\"op\":\"pong\"}\n")
    else:
        conn.sendall(b"{\"AgentStatus\":{\"version\":\"9.9.9\",\"protocol_version\":99,\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}\n")
    conn.close()
' "$sock3" &
listener3_pid=$!
disown "$listener3_pid" 2>/dev/null

for _ in $(seq 1 20); do
    [[ -S "$sock3" ]] && break
    sleep 0.1
done

if [[ ! -S "$sock3" ]]; then
    cleanup "$listener3_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create third test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock3"

if __gpy_supervisor_is_running; then
    echo "FAIL: supervisor treats mismatched-protocol agent as running: expected nonzero return"
    test_result=1
else
    echo "PASS: supervisor treats mismatched-protocol agent as not running"
fi

cleanup "$listener3_pid"
rm -rf "$test_tmp_dir"

exit "$test_result"
