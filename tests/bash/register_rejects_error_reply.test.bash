#!/usr/bin/env bash
# tests/bash/register_rejects_error_reply.test.bash
#
# Regression test for #758: __gpy_register_with_agent must count a
# registration as successful only when the reply carries "status":"ok" (as
# fish does). An {"error":...} reply used to set __gpy_registered=1, so the
# shell stopped retrying and received no repaint doorbells.
#
# A Python Unix-socket listener plays the agent and answers every request
# (sourcing init may itself register once); it is killed on exit.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract (#650): exits 0 locally, fails under CI.
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
test_result=0
listener_pid=""

# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    [[ -n "$listener_pid" ]] && kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# run_case <name> <reply> <expected-rc> <expected-registered>
run_case() {
    local name="$1" reply="$2" want_rc="$3" want_reg="$4"
    local sock="$test_tmp_dir/$name.sock"
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
while True:
    conn, _ = s.accept()
    conn.recv(65536)
    conn.sendall(sys.argv[2].encode() + b"\n")
    conn.close()
' "$sock" "$reply" &
    listener_pid=$!
    disown "$listener_pid" 2>/dev/null
    for _ in $(seq 1 20); do
        [[ -S "$sock" ]] && break
        sleep 0.1
    done
    if [[ ! -S "$sock" ]]; then
        test_skip "could not create test Unix socket listener"
    fi

    local out
    out="$(
        export GPY_AGENT_SOCKET_PATH="$sock"
        # shellcheck source=bash/core/constants.bash
        source bash/core/constants.bash
        # shellcheck source=bash/core/ipc.bash
        source bash/core/ipc.bash
        # shellcheck source=bash/core/init.bash
        source bash/core/init.bash >/dev/null 2>&1
        __gpy_track_shell_for_agent_recovery() { :; }
        __gpy_registered=""
        __gpy_register_with_agent
        echo "rc=$? registered=[$__gpy_registered]"
    )"
    kill -9 "$listener_pid" 2>/dev/null
    listener_pid=""

    if [[ "$out" == *"rc=$want_rc registered=[$want_reg]" ]]; then
        echo "PASS: $name"
    else
        echo "FAIL: $name: want rc=$want_rc registered=[$want_reg], got '$out'"
        test_result=1
    fi
}

run_case error_reply '{"error":"x"}' 1 ""
run_case ok_reply '{"status":"ok"}' 0 1

exit "$test_result"
