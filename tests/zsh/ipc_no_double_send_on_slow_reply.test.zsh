#!/usr/bin/env zsh
# tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh
#
# Regression test for #575: a slow-but-alive agent must not get the same
# request sent to it twice. __gpy_send_json's zsocket read has a 0.1s
# timeout; before the fix, a nonzero read_status from that timeout (agent
# connected fine, just answering slowly) fell straight through into the
# socat/nc fallback, which re-sent the *same* JSON payload over a brand new
# connection and waited again. One logical request became two on the wire.
#
# The "slow-but-alive agent" is simulated with a Python listener that
# accepts a connection, records it (so the test can count exactly how many
# connections were made), sleeps 200ms -- comfortably past the zsocket
# read's 100ms timeout -- then sends a complete reply and keeps listening
# for a bounded extra window. If the bug were present, that second window
# is exactly where the socat/nc re-send would land as a second recorded
# connection; the fix must leave it recording only one.

ROOT=${0:a:h:h:h}
# Shared skip contract (#650): exits 0 locally, fails under CI.
emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT"

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a slow-to-reply agent"
fi

test_tmp_dir=$(mktemp -d)
test_result=0

cleanup() {
    kill -9 "$1" 2>/dev/null
}

sock="$test_tmp_dir/gpy-slow-reply-test.sock"
count_file="$test_tmp_dir/connection-count"
reply_delay_secs=0.2
listener_lifetime_secs=2.0

# Accepts connections in a loop until listener_lifetime_secs elapses (or the
# listening socket's accept() itself times out), recording one line per
# accepted connection before replying -- so a fixed client (one connection)
# and a buggy client (a zsocket connection plus a socat/nc re-send) are
# distinguishable purely by counting lines afterward, without relying on
# timing alone.
python3 -c '
import socket, sys, time

sock_path, count_path, reply_delay, lifetime = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sock_path)
s.listen(5)

deadline = time.time() + lifetime
while True:
    remaining = deadline - time.time()
    if remaining <= 0:
        break
    s.settimeout(remaining)
    try:
        conn, _ = s.accept()
    except socket.timeout:
        break

    with open(count_path, "a") as f:
        f.write("connection\n")

    time.sleep(reply_delay)
    try:
        conn.sendall(b"{\"status\":\"ok\"}\n")
    except OSError:
        pass
    conn.close()
' "$sock" "$count_file" "$reply_delay_secs" "$listener_lifetime_secs" &
listener_pid=$!
disown 2>/dev/null

for _ in {1..20}; do
    [[ -S "$sock" ]] && break
    sleep 0.1
done

if [[ ! -S "$sock" ]]; then
    cleanup "$listener_pid"
    rm -rf "$test_tmp_dir"
    test_skip "could not create test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock"
result="$(__gpy_send_json '{"op":"ping"}')"

# __gpy_send_json is synchronous, so any fallback re-send it was going to
# make has already happened by the time it returns. This is just a small
# safety margin before reading the connection count.
sleep 0.3

cleanup "$listener_pid"
# Give the killed listener process a moment to actually exit so its last
# recorded connection (if any) is fully flushed to disk before we read it.
sleep 0.1

connection_count=0
if [[ -f "$count_file" ]]; then
    connection_count=$(wc -l < "$count_file" | tr -d ' ')
fi

rm -rf "$test_tmp_dir"

# Primary assertion (the acceptance criterion itself): the slow reply must
# not have caused a second request over a different connection.
if [[ "$connection_count" -eq 1 ]]; then
    echo "PASS: exactly one connection was made for one logical request"
else
    echo "FAIL: expected exactly one connection, got $connection_count"
    test_result=1
fi

# Secondary assertion: __gpy_send_json should give up at its own 100ms
# zsocket read timeout rather than return a stale response fetched via a
# fallback re-send.
if [[ -z "$result" ]]; then
    echo "PASS: __gpy_send_json returned no response, as expected when the reply arrives after its read timeout"
else
    echo "FAIL: __gpy_send_json returned a response instead of timing out: got '$result'"
    test_result=1
fi

exit "$test_result"
