#!/usr/bin/env fish
# Regression test for #764: a registered shell must keep its registration when
# a workspace sync returns an error other than "not registered" (e.g. `cd /etc`
# rejected by the agent's path validator). Dropping it made every later `cd`
# skip the sync while the agent still held the PID with its old cwd.
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

print_test_header "Workspace error keeps registration (#764)"

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

for i in (seq 1 30)
    test -S $sock; and break
    sleep 0.1
end
if not test -S $sock
    test_skip "could not create fake listener socket"
end

set -gx GPY_AGENT_SOCKET_PATH $sock
set -e GPY_SUPERVISOR_CHILD
set -l test_result PASS

# Drive __gpy_sync_workspace explicitly so each cd is exactly one sync.
functions -e __gpy_workspace_on_pwd

function __ws_count --argument-names op
    count (string match -- "$op *" < $__ws_tmp/ops.log)
end

function __ws_check --argument-names name ok detail
    if test "$ok" = 1
        print_test_result $name PASS
    else
        print_test_result $name FAIL $detail
        set -g __ws_failed 1
    end
end

cd $__ws_tmp/proj
__gpy_register_with_agent
__ws_check "initial register succeeds" (set -q __gpy_registered; and echo 1; or echo 0) "not registered"
__ws_check "initial register sent once" (test (__ws_count register) = 1; and echo 1; or echo 0) "registers=$(__ws_count register)"

# 1. Error reply (path rejected): registration and last workspace untouched.
cd $__ws_tmp/denied
set -l before_ws $__gpy_last_workspace
__gpy_sync_workspace
__ws_check "error reply keeps __gpy_registered" (set -q __gpy_registered; and echo 1; or echo 0) "registration dropped"
__ws_check "error reply does not re-register" (test (__ws_count register) = 1; and echo 1; or echo 0) "registers=$(__ws_count register)"
__ws_check "error reply leaves __gpy_last_workspace untouched" (test "$__gpy_last_workspace" = "$before_ws"; and echo 1; or echo 0) "last_workspace=$__gpy_last_workspace"

# 2. Next valid cd syncs normally and records the workspace.
cd $__ws_tmp/other
__gpy_sync_workspace
__ws_check "next cd sends a workspace op" (test (__ws_count workspace) = 2; and echo 1; or echo 0) "workspace ops=$(__ws_count workspace)"
__ws_check "next cd records __gpy_last_workspace" (test "$__gpy_last_workspace" = "$__ws_tmp/other"; and echo 1; or echo 0) "last_workspace=$__gpy_last_workspace"

# 3. "not registered" still clears and re-registers.
cd $__ws_tmp/lost
__gpy_sync_workspace
__ws_check "'not registered' triggers re-register" (test (__ws_count register) = 2; and echo 1; or echo 0) "registers=$(__ws_count register)"
__ws_check "re-register restores registration" (set -q __gpy_registered; and echo 1; or echo 0) "not registered after re-register"

cd /
if set -q __ws_failed
    set test_result FAIL
end
print_test_footer "Workspace error keeps registration" $test_result
test "$test_result" = PASS
