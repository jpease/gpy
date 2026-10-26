#!/usr/bin/env fish
# A registered shell must keep its registration when a workspace sync returns
# an error other than "not registered" (e.g. `cd /etc` rejected by the agent's
# path validator). Dropping it would make every later `cd` skip the sync while
# the agent still held the PID with its old cwd.
#
# A python3 fake listener stands in for the agent (the real agent is never
# started): `workspace`/`register` -> error for a cwd named `denied` (the real
# validator rejects the re-register from /etc too), "not registered" for a
# `workspace` from `lost`, ok otherwise. Every op is appended to a log so
# the test can count registers. Paths reach python only through argv.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Workspace error keeps registration"

if not command -q python3
    test_skip "python3 not installed, cannot run a fake listener"
end

set -g __ws_tmp (mktemp -d)
set -g __ws_listener_pid
function __ws_cleanup --on-event fish_exit
    test -n "$__ws_listener_pid"; and kill $__ws_listener_pid 2>/dev/null
    rm -rf $__ws_tmp
end

set -l sock $__ws_tmp/agent.sock
set -l oplog $__ws_tmp/ops.log
touch $oplog
mkdir -p $__ws_tmp/proj $__ws_tmp/denied $__ws_tmp/lost $__ws_tmp/other

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
' $sock $oplog >/dev/null 2>&1 &
set -g __ws_listener_pid $last_pid

poll_until 3 test -S $sock
if not test -S $sock
    test_skip "could not create fake listener socket"
end

set -gx GPY_AGENT_SOCKET_PATH $sock
# Registration writes the shell PID file under the runtime root; keep it in
# the sandbox instead of the caller's XDG_RUNTIME_DIR or ~/.cache.
set -gx XDG_CACHE_HOME $__ws_tmp/cache
set -e XDG_RUNTIME_DIR
set -e GPY_SUPERVISOR_CHILD

# Drive __gpy_sync_workspace explicitly so each cd is exactly one sync.
functions -e __gpy_workspace_on_pwd

function __ws_count --argument-names op
    count (string match -- "$op *" < $__ws_tmp/ops.log)
end

cd $__ws_tmp/proj
__gpy_register_with_agent
check "initial register succeeds" 1 (set -q __gpy_registered; and echo 1; or echo 0)
check "initial register sent once" 1 (__ws_count register)

# 1. Error reply (path rejected): registration and last workspace untouched.
cd $__ws_tmp/denied
set -l before_ws $__gpy_last_workspace
__gpy_sync_workspace
check "error reply keeps __gpy_registered" 1 (set -q __gpy_registered; and echo 1; or echo 0)
check "error reply does not re-register" 1 (__ws_count register)
check "error reply leaves __gpy_last_workspace untouched" "$before_ws" "$__gpy_last_workspace"

# 2. Next valid cd syncs normally and records the workspace.
cd $__ws_tmp/other
__gpy_sync_workspace
check "next cd sends a workspace op" 2 (__ws_count workspace)
check "next cd records __gpy_last_workspace" "$__ws_tmp/other" "$__gpy_last_workspace"

# 2b. Fish syncs once per cd (never per prompt); cd away and back re-sends.
cd $__ws_tmp/denied
__gpy_sync_workspace
check "cd back into rejected dir re-sends" 3 (__ws_count workspace)

# 3. "not registered" still clears and re-registers.
cd $__ws_tmp/lost
__gpy_sync_workspace
check "'not registered' triggers re-register" 2 (__ws_count register)
check "re-register restores registration" 1 (set -q __gpy_registered; and echo 1; or echo 0)

cd /
test_finish "Workspace error keeps registration"
