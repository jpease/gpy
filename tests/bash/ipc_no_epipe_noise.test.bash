#!/usr/bin/env bash
# tests/bash/ipc_no_epipe_noise.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A complete IPC reply must not make the shell print "write error: Broken pipe"
# when SIGPIPE is ignored (#848).
#
# __gpy_send_json reads the agent's reply through `IFS= read -r response <
# <(CLIENT; printf '\037%s\n' "$?")`. `read` returns -- and closes its end of
# the pipe -- as soon as the reply line arrives, so the trailer `printf` that
# follows writes to a closed pipe. With the default SIGPIPE disposition that
# kills the subshell silently. With SIGPIPE ignored, which every shell
# spawned by Python, Node, .NET (each GitHub Actions step) or many IDE
# terminals inherits, the write fails with EPIPE and bash printed
# "printf: write error: Broken pipe" into the prompt, once per request: three
# lines per prompt on the first Linux CI run, failing every e2e assertion on
# "error output".
#
# The client runs through a Python parent that ignores SIGPIPE and execs bash
# with the disposition inherited (restore_signals=False). A listener serves a
# complete, newline-terminated reply to each connection. Several requests are
# made so a lost race on one of them is still seen.
#
# Run against the unfixed ipc.bash this test fails with the printf message on
# stderr; with `exec 2>/dev/null` opening each substitution it passes.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Shared skip contract (#650): exits 0 locally, fails under CI.
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

for _ in $(seq 1 50); do
    [[ -S "$sock" ]] && break
    sleep 0.1
done
if [[ ! -S "$sock" ]]; then
    echo "FAIL: the listener never bound $sock"
    exit 1
fi

# The client: bash with SIGPIPE inherited as ignored, its stdout and stderr
# captured separately.
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

failures=0
replies="$(grep -c '^{"status":"ok"}$' "$scratch/out" 2>/dev/null || true)"
if [[ "${replies:-0}" -eq "$requests" ]]; then
    echo "PASS: all $requests complete replies round-tripped"
else
    echo "FAIL: expected $requests replies, got ${replies:-0}:"
    sed 's/^/    /' "$scratch/out"
    failures=$((failures + 1))
fi

if [[ -s "$scratch/err" ]]; then
    echo "FAIL: the IPC client wrote to stderr with SIGPIPE ignored:"
    sed 's/^/    /' "$scratch/err"
    failures=$((failures + 1))
else
    echo "PASS: no stderr output with SIGPIPE ignored"
fi

if [[ "$failures" -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: a complete reply leaves stderr clean when SIGPIPE is ignored"
