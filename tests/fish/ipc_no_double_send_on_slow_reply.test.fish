#!/usr/bin/env fish
# The Fish twin of tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh (#845).
#
# One slow-reply policy across the shells: an agent that accepted the request
# but answers late is working on it. The shell must (a) put the request on the
# wire exactly once -- no resend over a second connection (#575) -- and (b)
# report "connected, no reply" (status 2, empty output) so no caller recomputes
# the same segment with a blocking `gpy-agent oneshot` fork (#757). Before #845
# Fish reported status 1 and __gpy_request forked oneshot on every late reply.
# Only an agent that cannot be reached at all (status 1) may fall back.
#
# A python3 listener plays the slow agent and records one line per accepted
# connection. Each transport that exists on this machine is exercised on its
# own, by putting just that client on PATH: socat, nc without `timeout` (nc's
# own whole-second -w bound), and nc wrapped in `timeout` when that exists.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC slow-reply policy (#845)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a slow-to-reply agent"
end

set -g tmp (mktemp -d)
set -g listener_pid
set -g failures 0
set -g real_path $PATH

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

# make_shim NAME TOOL...: a directory holding only the named tools.
function make_shim --argument-names name
    set -l dir $tmp/shim-$name
    mkdir -p $dir
    for tool in $argv[2..-1]
        ln -sf (command -v $tool) $dir/$tool
    end
    echo $dir
end

# start_listener DELAY LIFETIME: replies `{"status":"ok"}` after DELAY seconds,
# counting accepted connections in $tmp/connections.
function start_listener --argument-names delay lifetime
    rm -f $tmp/slow.sock $tmp/connections
    python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1]); s.listen(5)
deadline = time.time() + float(sys.argv[4])
while True:
    left = deadline - time.time()
    if left <= 0:
        break
    s.settimeout(left)
    try:
        c, _ = s.accept()
    except socket.timeout:
        break
    with open(sys.argv[2], "a") as f:
        f.write("connection\n")
    c.recv(65536)
    time.sleep(float(sys.argv[3]))
    try:
        c.sendall(b"{\"status\":\"ok\"}\n")
    except OSError:
        pass
    c.close()
' $tmp/slow.sock $tmp/connections $delay $lifetime &
    set -g listener_pid $last_pid
    for i in (seq 1 30)
        test -S $tmp/slow.sock; and return 0
        sleep 0.1
    end
    finish
    test_skip "could not create test Unix socket listener"
end

function stop_listener
    kill -9 $listener_pid 2>/dev/null
    set -g listener_pid
end

# check_transport LABEL PATH DELAY: one slow request over the given PATH.
function check_transport --argument-names label use_path delay
    start_listener $delay 6
    set -gx GPY_AGENT_SOCKET_PATH $tmp/slow.sock
    set -g GPY_IPC_TIMEOUT_MS 150
    set -gx PATH $use_path
    set -l out (__gpy_ipc_send '{"op":"ping"}')
    set -l rc $status
    set -gx PATH $real_path
    sleep 0.3
    stop_listener

    set -l connections 0
    test -f $tmp/connections; and set connections (wc -l <$tmp/connections | string trim)
    check "$label: one logical request is one connection" (test "$connections" = 1; and echo 1; or echo 0) "expected exactly one connection, got $connections"
    check "$label: a connected-but-late agent is status 2 with no output" (test "$rc" = 2 -a -z "$out"; and echo 1; or echo 0) "status $rc, output '$out' (want status 2, empty)"
end

set -l ran 0
if command -q socat
    set ran (math $ran + 1)
    check_transport socat (make_shim socat socat) 0.5
end
if command -q nc
    set -l nc_shim (make_shim nc nc)
    set -gx PATH $nc_shim
    __gpy_nc_probe_capabilities
    set -gx PATH $real_path
    if test "$__gpy_nc_supports_unix" = 1
        set ran (math $ran + 1)
        # nc's -w is whole seconds, so the reply has to be later than 1s.
        check_transport "nc (no timeout)" $nc_shim 1.6
        if command -q timeout
            set ran (math $ran + 1)
            check_transport "nc (timeout wrapper)" (make_shim nct nc timeout) 0.5
        end
    end
end
if test $ran -eq 0
    finish
    test_skip "neither socat nor an nc with -U is installed"
end

# The caller-visible half: __gpy_request never forks oneshot for a late agent,
# and still does for one that cannot be reached. A stub gpy-agent logs calls.
mkdir -p $tmp/bin $tmp/cache
set -g oneshot_log $tmp/oneshot.log
printf '%s\n' '#!/bin/sh' 'echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"' 'echo ONESHOT_OUTPUT' >$tmp/bin/gpy-agent
chmod +x $tmp/bin/gpy-agent
set -gx GPY_TEST_ONESHOT_LOG $oneshot_log
set -gx XDG_CACHE_HOME $tmp/cache
set -g GPY_AGENT_BINARY_PATH $tmp/bin/gpy-agent

function run_request --argument-names socket
    : >$oneshot_log
    set -e __gpy_oneshot_used
    set -gx GPY_AGENT_SOCKET_PATH $socket
    __gpy_request git $tmp true "" true
end

start_listener 0.6 6
set -l out (run_request $tmp/slow.sock)
stop_listener
check "__gpy_request omits the segment for a late agent and forks no oneshot" (test -z "$out" -a ! -s $oneshot_log; and echo 1; or echo 0) "output '$out', oneshot log '"(cat $oneshot_log)"'"

set out (run_request $tmp/missing.sock)
check "__gpy_request still falls back to oneshot when the socket is missing" (test "$out" = ONESHOT_OUTPUT -a -s $oneshot_log; and echo 1; or echo 0) "output '$out', oneshot log '"(cat $oneshot_log)"'"

# A socket file nobody listens on (a crashed agent): connection refused is
# "unreachable", not "late", so it must also fall back.
python3 -c 'import socket, sys; s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()' $tmp/dead.sock
set out (run_request $tmp/dead.sock)
check "__gpy_request still falls back to oneshot when nothing listens on the socket" (test "$out" = ONESHOT_OUTPUT -a -s $oneshot_log; and echo 1; or echo 0) "output '$out', oneshot log '"(cat $oneshot_log)"'"

finish
if test $failures -gt 0
    print_test_footer "IPC slow-reply policy" FAIL
    exit 1
end
print_test_footer "IPC slow-reply policy" PASS
