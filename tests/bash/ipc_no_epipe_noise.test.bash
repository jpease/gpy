#!/usr/bin/env bash
# tests/bash/ipc_no_epipe_noise.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A complete IPC reply must not make the shell print "write error: Broken pipe"
# when SIGPIPE is ignored.
# `read` closes the pipe early, so the trailer `printf` gets EPIPE when SIGPIPE
# is ignored (inherited from Python/Node/CI). Repeated to catch a lost race.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command python3
if ! command -v socat >/dev/null 2>&1 && ! command -v nc >/dev/null 2>&1; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpy-epipe.XXXXXX")"
listener_pid=""
cleanup() {
    [[ -n "$listener_pid" ]] && kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$scratch"
}
trap cleanup EXIT

sock="$scratch/gpy.sock"
requests=8
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(8)
for _ in range(int(sys.argv[2])):
    conn, _ = s.accept()
    conn.recv(4096)
    conn.sendall(b"{\"status\":\"ok\"}\n")
    conn.close()
' "$sock" "$requests" &
listener_pid=$!
disown "$listener_pid" 2>/dev/null

if ! shell_e2e_poll 5 test -S "$sock"; then
    echo "FAIL: the listener never bound $sock"
    exit 1
fi

# The single quotes are deliberate: the code runs in the child bash, not here.
# shellcheck disable=SC2016
client='source bash/core/ipc.bash
for _ in $(seq 1 '"$requests"'); do
    __gpy_send_json "{\"op\":\"ping\"}"
done'
GPY_AGENT_SOCKET_PATH="$sock" python3 - "$client" "$scratch/out" "$scratch/err" <<'PY'
import signal, subprocess, sys
signal.signal(signal.SIGPIPE, signal.SIG_IGN)
client, out, err = sys.argv[1:4]
with open(out, "wb") as o, open(err, "wb") as e:
    subprocess.run(["bash", "-c", client], stdout=o, stderr=e, restore_signals=False)
PY

replies="$(grep -c '^{"status":"ok"}$' "$scratch/out" 2>/dev/null || true)"
if [[ "${replies:-0}" -eq "$requests" ]]; then
    pass "all $requests complete replies round-tripped"
else
    fail "expected $requests replies, got ${replies:-0}:"
    sed 's/^/    /' "$scratch/out"
fi

if [[ -s "$scratch/err" ]]; then
    fail "the IPC client wrote to stderr with SIGPIPE ignored:"
    sed 's/^/    /' "$scratch/err"
else
    pass "no stderr output with SIGPIPE ignored"
fi

test_finish "a complete reply leaves stderr clean when SIGPIPE is ignored"
