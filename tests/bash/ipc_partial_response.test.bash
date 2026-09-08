#!/usr/bin/env bash
# tests/bash/ipc_partial_response.test.bash
#
# Regression test for #300: a truncated (partial-write) IPC response must
# never reach the caller as if it were complete. __gpy_send_json previously
# treated any non-empty read as a valid response, so an agent that wrote
# `{"status":"o` and then stalled or died mid-write would have that exact
# truncated string handed to __gpy_request_character/_duration (raw ANSI,
# echoed straight into the prompt) — corrupting terminal rendering.
#
# The "wedged-mid-write agent" is simulated with a tiny Python listener that
# accept()s the connection, writes a partial (non-newline-terminated) frame,
# then goes silent. The listener self-destructs after a few seconds so this
# test has a hard, deterministic upper bound even if the fix regresses and
# the client hangs waiting for more bytes.
#
# A second case guards against the fix being too strict: a normally-terminated
# complete response must still be returned intact.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract (#650): exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a partial-write agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
test_result=0

cleanup() {
    kill -9 "$1" 2>/dev/null
}

# --- Case 1: partial write then stall must not leak the truncated bytes ---

sock1="$test_tmp_dir/gpy-partial-test.sock"
listener_lifetime_secs=3
python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.sendall(b"{\"status\":\"o")
time.sleep(float(sys.argv[2]))
' "$sock1" "$listener_lifetime_secs" &
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
result="$(__gpy_send_json '{"op":"ping"}')"

cleanup "$listener1_pid"

if [[ -n "$result" ]]; then
    echo "FAIL: partial write is discarded, not returned: got '$result'"
    test_result=1
else
    echo "PASS: partial write is discarded, not returned"
fi

# --- Case 2: a complete, newline-terminated response still round-trips ---

sock2="$test_tmp_dir/gpy-complete-test.sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.sendall(b"{\"status\":\"ok\"}\n")
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
complete_result="$(__gpy_send_json '{"op":"ping"}')"

cleanup "$listener2_pid"
rm -rf "$test_tmp_dir"

if [[ "$complete_result" == '{"status":"ok"}' ]]; then
    echo "PASS: complete response still round-trips"
else
    echo "FAIL: complete response still round-trips: got '$complete_result'"
    test_result=1
fi

exit "$test_result"
