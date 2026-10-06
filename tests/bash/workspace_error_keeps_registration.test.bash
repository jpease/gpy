#!/usr/bin/env bash
# tests/bash/workspace_error_keeps_registration.test.bash
#
# Regression test for #764 (sibling of the fish fix): __gpy_sync_workspace must
# keep the registration when the workspace reply is an error other than
# "not registered" (e.g. `cd /etc` rejected by the path validator). It used to
# clear __gpy_registered, the re-register from the same rejected directory
# failed too, and every later cd skipped the sync.
#
# A python3 listener plays the agent: `workspace`/`register` from a directory
# named `denied` -> error; `workspace` from `lost` -> "not registered"; else
# ok. Every op is logged. The agent is never started; the listener is killed
# on exit. Paths reach python only through argv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
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
listener_pid=""
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    [[ -n "$listener_pid" ]] && kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

sock="$test_tmp_dir/agent.sock"
oplog="$test_tmp_dir/ops.log"
: >"$oplog"
mkdir -p "$test_tmp_dir/proj" "$test_tmp_dir/denied" "$test_tmp_dir/lost" "$test_tmp_dir/other"

python3 -c '
import json, os, socket, sys
sock_path, log_path = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sock_path)
s.listen(4)
while True:
    conn, _ = s.accept()
    with conn:
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            buf += chunk
        try:
            msg = json.loads(buf.decode())
        except ValueError:
            continue
        op, cwd = msg.get("op"), msg.get("cwd", "")
        with open(log_path, "a") as log:
            log.write(op + " " + cwd + "\n")
        name = os.path.basename(cwd)
        if op in ("workspace", "register") and name == "denied":
            reply = {"error": "Invalid workspace path: Access to system directories denied"}
        elif op == "workspace" and name == "lost":
            reply = {"error": "PID 123 not registered - please register first"}
        else:
            reply = {"status": "ok"}
        conn.sendall((json.dumps(reply, separators=(",", ":")) + "\n").encode())
' "$sock" "$oplog" &
listener_pid=$!
disown "$listener_pid" 2>/dev/null
for _ in $(seq 1 30); do
    [[ -S "$sock" ]] && break
    sleep 0.1
done
[[ -S "$sock" ]] || test_skip "could not create test Unix socket listener"

test_result=0
check() {
    if [[ "$2" == "$3" ]]; then
        echo "PASS: $1"
    else
        echo "FAIL: $1: want [$2], got [$3]"
        test_result=1
    fi
}
count_ops() { grep -c "^$1 " "$oplog"; }

export GPY_AGENT_SOCKET_PATH="$sock"
# shellcheck source=bash/core/constants.bash
source bash/core/constants.bash
# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash
# shellcheck source=bash/core/init.bash
source bash/core/init.bash >/dev/null 2>&1
__gpy_track_shell_for_agent_recovery() { :; }
: >"$oplog"
__gpy_registered=""
__gpy_last_workspace=""

cd "$test_tmp_dir/proj" || exit 1
__gpy_register_with_agent
check "initial register" "1" "$__gpy_registered"

cd "$test_tmp_dir/denied" || exit 1
before_ws="$__gpy_last_workspace"
__gpy_sync_workspace
check "error reply keeps __gpy_registered" "1" "$__gpy_registered"
check "error reply does not re-register" "1" "$(count_ops register)"
check "error reply leaves __gpy_last_workspace untouched" "$before_ws" "$__gpy_last_workspace"

cd "$test_tmp_dir/other" || exit 1
__gpy_sync_workspace
check "next cd sends a workspace op" "2" "$(count_ops workspace)"
check "next cd records __gpy_last_workspace" "$test_tmp_dir/other" "$__gpy_last_workspace"

cd "$test_tmp_dir/lost" || exit 1
__gpy_sync_workspace
check "'not registered' re-registers" "2" "$(count_ops register)"
check "re-register restores registration" "1" "$__gpy_registered"

exit "$test_result"
