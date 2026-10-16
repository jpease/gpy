#!/usr/bin/env bash
# tests/bash/ipc_partial_response.test.bash
#
# A truncated IPC response must never reach the caller as if complete; a
# complete response must round-trip, preserving whitespace and backslashes.
# The listener self-destructs, bounding the test if the client hangs.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract: exits 0 locally, fails under CI.
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

cleanup() {
    kill -9 "$1" 2>/dev/null
}


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

if ! shell_e2e_poll 2 test -S "$sock1"; then
    cleanup "$listener1_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock1"
result="$(__gpy_send_json '{"op":"ping"}')"

cleanup "$listener1_pid"

if [[ -n "$result" ]]; then
    fail "partial write is discarded, not returned: got '$result'"
else
    pass "partial write is discarded, not returned"
fi


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

if ! shell_e2e_poll 2 test -S "$sock2"; then
    cleanup "$listener2_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create second test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock2"
complete_result="$(__gpy_send_json '{"op":"ping"}')"

cleanup "$listener2_pid"

if [[ "$complete_result" == '{"status":"ok"}' ]]; then
    pass "complete response still round-trips"
else
    fail "complete response still round-trips: got '$complete_result'"
fi

# Fragments end in a space and the clock embeds `\D{..}`; IFS splitting or
# backslash mangling would change the prompt.

sock3="$test_tmp_dir/gpy-whitespace-test.sock"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.sendall(b"  {\"status\":\"ok\",\"text\":\"\\\\D{%H} \"}  \n")
conn.close()
' "$sock3" &
listener3_pid=$!
disown "$listener3_pid" 2>/dev/null

if ! shell_e2e_poll 2 test -S "$sock3"; then
    cleanup "$listener3_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create third test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock3"
ws_result="$(__gpy_send_json '{"op":"ping"}')"

cleanup "$listener3_pid"
rm -rf "$test_tmp_dir"

ws_want='  {"status":"ok","text":"\\D{%H} "}  '
if [[ "$ws_result" == "$ws_want" ]]; then
    pass "leading/trailing whitespace and backslashes are preserved"
else
    fail "whitespace/backslashes preserved: got '$ws_result', want '$ws_want'"
fi

test_finish
