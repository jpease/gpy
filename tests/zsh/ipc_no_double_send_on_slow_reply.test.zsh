#!/usr/bin/env zsh
# tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh
#
# A connected agent that has not replied must
# get exactly one request, and __gpy_send_json must return status 2 with no
# output when its response budget expires.
#
# Do not schedule a reply a fixed interval after accept(): the client can be
# descheduled between connecting and entering read -t, making even a "late"
# reply available before its read budget begins. Instead the listener
# holds every accepted socket open without replying until the synchronous
# send returns. A control FIFO then tells it to drain pending connections and
# record the requests, including any socat/nc fallback re-send. No sleep decides
# whether the response timed out or how many requests reached the listener.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/ipc.zsh

test_require_command python3 "python3 not installed, cannot simulate a slow-to-reply agent"
if ! zmodload zsh/net/socket 2>/dev/null; then
    test_skip "zsh/net/socket not available, cannot exercise the builtin transport"
fi

test_tmp_dir=$(mktemp -d)
listener_pid=""
control_fd=""

cleanup() {
    if [[ -n "$listener_pid" ]]; then
        kill "$listener_pid" 2>/dev/null
        wait "$listener_pid" 2>/dev/null
    fi
    [[ -n "$control_fd" ]] && exec {control_fd}>&-
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

sock="$test_tmp_dir/gpy.sock"
count_file="$test_tmp_dir/connection-count"
request_file="$test_tmp_dir/requests"
ready_file="$test_tmp_dir/ready"
control_file="$test_tmp_dir/control"
mkfifo "$control_file"
# Keep both FIFO ends open so startup and a failed listener cannot block the
# completion notification. Only the listener reads the notification.
exec {control_fd}<>"$control_file"

python3 -c '
import os, select, socket, sys, time

sock_path, count_path, request_path, ready_path, control_path = sys.argv[1:]
connections = []
control = os.open(control_path, os.O_RDONLY | os.O_NONBLOCK)

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
    listener.bind(sock_path)
    listener.listen(5)
    listener.setblocking(False)
    with open(ready_path, "w") as ready:
        ready.write("ready\n")

    def accept_pending():
        while True:
            try:
                conn, _ = listener.accept()
            except BlockingIOError:
                return
            connections.append(conn)

    deadline = time.monotonic() + 10
    try:
        while True:
            # This is only a fixture watchdog, never the response delay. If
            # it fires, the listener fails rather than making EOF look like
            # the response timeout being tested.
            remaining = max(0, deadline - time.monotonic())
            readable, _, _ = select.select([listener, control], [], [], remaining)
            if not readable:
                raise RuntimeError("fixture watchdog expired before the client returned")
            accept_pending()
            if control in readable:
                if os.read(control, 65536) != b"done\n":
                    raise RuntimeError("invalid client completion notification")
                # The synchronous client is finished: any fallback connect
                # has already happened, even if this process was descheduled.
                accept_pending()
                break

        requests = []
        for conn in connections:
            conn.settimeout(1)
            chunks = []
            while True:
                chunk = conn.recv(65536)
                if not chunk:
                    break
                chunks.append(chunk)
            requests.append(b"".join(chunks))
        with open(count_path, "w") as counts:
            counts.write("connection\n" * len(connections))
        with open(request_path, "wb") as recorded:
            recorded.write(b"".join(requests))
    finally:
        for conn in connections:
            conn.close()
        os.close(control)
' "$sock" "$count_file" "$request_file" "$ready_file" "$control_file" &
listener_pid=$!

# Readiness is acknowledged after listen(), not merely after bind() creates
# the socket path.
listener_ready() { [[ -f "$ready_file" ]]; }
shell_e2e_poll 5 listener_ready
if [[ ! -f "$ready_file" ]]; then
    echo "FAIL: test Unix socket listener never became ready"
    exit 1
fi

# Pin the premise instead of inheriting a caller budget or agent-free mode.
export GPY_AGENT_SOCKET_PATH="$sock" GPY_IPC_TIMEOUT_MS=100 GPY_AGENT_ENABLED=1
result="$(__gpy_send_json '{"op":"ping"}')"
send_rc=$?

print -r -u "$control_fd" -- done
wait "$listener_pid"
listener_status=$?
listener_pid=""

# A clean completion proves the fixture stayed open without replying until
# the client returned; its own watchdog/EOF cannot satisfy the timeout check.
if (( listener_status != 0 )); then
    fail "slow-reply fixture exited with status $listener_status"
fi

connection_count=0
if [[ -f "$count_file" ]]; then
    connection_count=$(wc -l < "$count_file" | tr -d ' ')
fi

# Primary assertion: the slow reply must
# not have caused a second request over a different connection.
if [[ "$connection_count" -eq 1 ]]; then
    pass "exactly one connection was made for one logical request"
else
    fail "expected exactly one connection, got $connection_count"
fi

# Status 2 distinguishes an actual connected response timeout from a missing
# socket/client (status 1); the fixture has not sent a byte or closed a socket.
if (( send_rc == 2 )) && [[ -z "$result" ]]; then
    pass "__gpy_send_json timed out with status 2 and no response"
else
    fail "expected status 2 and no response, got status $send_rc and '$result'"
fi

# Count the request itself too, not just accepted sockets: an empty connection
# or two sends on the same connection must not pass the no-double-send check.
if printf '%s\n' '{"op":"ping"}' | cmp -s - "$request_file"; then
    pass "listener received exactly one complete ping request"
else
    fail "listener did not receive exactly one complete ping request"
fi

test_finish
