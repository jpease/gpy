#!/usr/bin/env fish
# The Fish twin of tests/zsh/ipc_no_double_send_on_slow_reply.test.zsh.
#
# One slow-reply policy across the shells: an agent that accepted the request
# but answers late is working on it. The shell must (a) put the request on the
# wire exactly once -- no resend over a second connection -- and (b) report
# "connected, no reply" (status 2, empty output) so no caller recomputes the
# same segment with a blocking `gpy-agent oneshot` fork. Only an agent that
# cannot be reached at all (status 1) may fall back.
#
# A stub listener plays the slow agent and records one line per accepted
# connection. Each transport that exists on this machine is exercised on its
# own, by putting just that client on PATH: socat, nc without `timeout` (nc's
# own whole-second -w bound), and nc wrapped in `timeout` when that exists.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC slow-reply policy"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a slow-to-reply agent"
end

set -g tmp (mktemp -d)
set -g real_path $PATH

function finish
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

# start_listener DELAY: replies `{"status":"ok"}` after DELAY seconds,
# counting accepted connections in $tmp/connections.
function start_listener --argument-names delay
    rm -f $tmp/slow.sock $tmp/connections
    test_stub_listener $tmp/slow.sock reply --reply '{"status":"ok"}' \
        --delay $delay --lifetime 6 --connections $tmp/connections
end

# check_transport LABEL PATH DELAY: one slow request over the given PATH.
function check_transport --argument-names label use_path delay
    start_listener $delay
    set -gx GPY_AGENT_SOCKET_PATH $tmp/slow.sock
    set -g GPY_IPC_TIMEOUT_MS 150
    set -gx PATH $use_path
    set -l out (__gpy_ipc_send '{"op":"ping"}')
    set -l rc $status
    set -gx PATH $real_path
    # Window in which a resend over a second connection would land; the
    # connection count below must still be exactly one after it.
    sleep 0.3
    test_stop_pids $test_stub_pid

    set -l connections 0
    test -f $tmp/connections; and set connections (wc -l <$tmp/connections | string trim)
    check "$label: one logical request is one connection" 1 "$connections"
    check "$label: a connected-but-late agent is status 2 with no output" 1 (test "$rc" = 2 -a -z "$out"; and echo 1; or echo 0)
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
set -gx PATH $tmp/bin $real_path

function run_request --argument-names socket
    : >$oneshot_log
    set -e __gpy_oneshot_used
    set -gx GPY_AGENT_SOCKET_PATH $socket
    __gpy_request git $tmp true "" true
end

start_listener 0.6
set -l out (run_request $tmp/slow.sock)
test_stop_pids $test_stub_pid
check "__gpy_request omits the segment for a late agent and forks no oneshot" 1 (test -z "$out" -a ! -s $oneshot_log; and echo 1; or echo 0)

set out (run_request $tmp/missing.sock)
check "__gpy_request still falls back to oneshot when the socket is missing" 1 (test "$out" = ONESHOT_OUTPUT -a -s $oneshot_log; and echo 1; or echo 0)

# A socket file nobody listens on (a crashed agent): connection refused is
# "unreachable", not "late", so it must also fall back.
test_stub_listener $tmp/dead.sock bind-only
set out (run_request $tmp/dead.sock)
check "__gpy_request still falls back to oneshot when nothing listens on the socket" 1 (test "$out" = ONESHOT_OUTPUT -a -s $oneshot_log; and echo 1; or echo 0)

finish
test_finish "IPC slow-reply policy"
