#!/usr/bin/env fish
# __gpy_socket_ready must not need coreutils `timeout` (#850).
#
# Stock macOS has no `timeout`, and the check used to be gated on it: without
# it the function skipped every probe and answered "ready" for any file that
# merely existed, so __gpy_wait_for_socket returned at once for a stale socket
# left behind by a crashed agent. Readiness is now a ping over the same client
# every real request uses (socat, or nc with -U), bounded by that client's own
# timeout flags.
#
# Each client that exists here is exercised alone on a PATH with no `timeout`:
#   - a listener that answers pings        -> ready;
#   - a socket file nobody listens on      -> not ready (the old false positive);
#   - a listener that never answers        -> not ready, and promptly;
#   - no socket file at all                -> not ready;
#   - no client on PATH at all             -> the file existing is all there is
#     to go on, so it counts as ready (documented fallback).

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Socket readiness without timeout (#850)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate an agent"
end

set -g tmp (mktemp -d)
set -g real_path $PATH
set -g failures 0
set -g listener_pid

function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        print_test_result "$label" FAIL "$detail"
        set -g failures (math $failures + 1)
    end
end

function finish
    test -n "$listener_pid"; and kill -9 $listener_pid 2>/dev/null
    set -gx PATH $real_path
    rm -rf $tmp
end

function make_shim --argument-names name
    set -l dir $tmp/shim-$name
    mkdir -p $dir
    for tool in $argv[2..-1]
        ln -sf (command -v $tool) $dir/$tool
    end
    echo $dir
end

# start_listener DELAY: answers every connection with a ping reply after DELAY
# seconds, for at most 8s.
function start_listener --argument-names delay
    rm -f $tmp/agent.sock
    python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1]); s.listen(8)
deadline = time.time() + 8
while True:
    left = deadline - time.time()
    if left <= 0:
        break
    s.settimeout(left)
    try:
        c, _ = s.accept()
    except socket.timeout:
        break
    c.settimeout(2)
    try:
        c.recv(65536)
        time.sleep(float(sys.argv[2]))
        c.sendall(b"{\"status\":\"ok\"}\n")
    except OSError:
        pass
    c.close()
' $tmp/agent.sock $delay &
    set -g listener_pid $last_pid
    for i in (seq 1 30)
        test -S $tmp/agent.sock; and return 0
        sleep 0.1
    end
    finish
    test_skip "could not create test Unix socket listener"
end

function stop_listener
    kill -9 $listener_pid 2>/dev/null
    set -g listener_pid
end

# probe LABEL SHIM: the four socket states, with only SHIM on PATH.
function probe --argument-names label shim
    set -g GPY_IPC_TIMEOUT_MS 150

    start_listener 0
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l live $status
    set -gx PATH $real_path
    stop_listener
    check "$label: a socket that answers pings is ready" (test $live -eq 0; and echo 1; or echo 0) "status $live"

    # The listener is gone but its socket file remains: a crashed agent.
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l stale $status
    set -gx PATH $real_path
    check "$label: a stale socket file is not ready" (test $stale -ne 0; and echo 1; or echo 0) "status $stale (the file exists but nothing answers)"

    start_listener 6
    set -l started (date +%s)
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l silent $status
    set -gx PATH $real_path
    set -l elapsed (math (date +%s) - $started)
    stop_listener
    check "$label: a socket that never answers is not ready, within the probe's bound" (test $silent -ne 0 -a $elapsed -le 3; and echo 1; or echo 0) "status $silent after ~"$elapsed"s"

    set -gx PATH $shim
    __gpy_socket_ready $tmp/absent.sock
    set -l absent $status
    set -gx PATH $real_path
    check "$label: a missing socket file is not ready" (test $absent -ne 0; and echo 1; or echo 0) "status $absent"
end

set -l ran 0
if command -q socat
    set ran (math $ran + 1)
    probe socat (make_shim socat socat sleep)
end
if command -q nc
    set -l nc_shim (make_shim nc nc sleep)
    set -gx PATH $nc_shim
    __gpy_nc_probe_capabilities
    set -gx PATH $real_path
    if test "$__gpy_nc_supports_unix" = 1
        set ran (math $ran + 1)
        probe nc $nc_shim
    end
end
if test $ran -eq 0
    finish
    test_skip "neither socat nor an nc with -U is installed"
end

# No client at all: nothing can be probed, so an existing socket file is ready.
start_listener 0
set -l empty_shim (make_shim none)
set -gx PATH $empty_shim
set -e __gpy_nc_probed
__gpy_socket_ready $tmp/agent.sock
set -l no_client $status
set -gx PATH $real_path
stop_listener
check "with no client on PATH an existing socket file counts as ready" (test $no_client -eq 0; and echo 1; or echo 0) "status $no_client"

finish
if test $failures -gt 0
    print_test_footer "Socket readiness without timeout" FAIL
    exit 1
end
print_test_footer "Socket readiness without timeout" PASS
