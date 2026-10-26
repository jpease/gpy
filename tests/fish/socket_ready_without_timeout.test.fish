#!/usr/bin/env fish
# __gpy_socket_ready must not need coreutils `timeout`.
#
# Stock macOS has no `timeout`. A check gated on it would skip every probe
# there and answer "ready" for any file that merely exists, so
# __gpy_wait_for_socket would return at once for a stale socket left behind
# by a crashed agent. Readiness is a ping over the same client every real
# request uses (socat, or nc with -U), bounded by that client's own timeout
# flags.
#
# Each client that exists here is exercised alone on a PATH with no `timeout`:
#   - a listener that answers pings        -> ready;
#   - a socket file nobody listens on      -> not ready;
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

print_test_header "Socket readiness without timeout"

if not command -q python3
    test_skip "python3 not installed, cannot simulate an agent"
end

set -g tmp (mktemp -d)
set -g real_path $PATH

function finish
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

# start_listener MODE [OPTION...]: a stub agent on $tmp/agent.sock for at
# most 8s.
function start_listener
    rm -f $tmp/agent.sock
    test_stub_listener $tmp/agent.sock $argv --lifetime 8
end

# probe LABEL SHIM: the four socket states, with only SHIM on PATH.
function probe --argument-names label shim
    set -g GPY_IPC_TIMEOUT_MS 150

    start_listener reply --reply '{"status":"ok"}'
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l live $status
    set -gx PATH $real_path
    test_stop_pids $test_stub_pid
    check "$label: a socket that answers pings is ready" 0 $live

    # The listener is gone but its socket file remains: a crashed agent.
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l stale $status
    set -gx PATH $real_path
    check "$label: a stale socket file is not ready" 1 (test $stale -ne 0; and echo 1; or echo 0)

    start_listener silent
    set -l started (date +%s)
    set -gx PATH $shim
    __gpy_socket_ready $tmp/agent.sock
    set -l silent $status
    set -gx PATH $real_path
    set -l elapsed (math (date +%s) - $started)
    test_stop_pids $test_stub_pid
    check "$label: a socket that never answers is not ready, within the probe's bound" 1 (test $silent -ne 0 -a $elapsed -le 3; and echo 1; or echo 0)

    set -gx PATH $shim
    __gpy_socket_ready $tmp/absent.sock
    set -l absent $status
    set -gx PATH $real_path
    check "$label: a missing socket file is not ready" 1 (test $absent -ne 0; and echo 1; or echo 0)
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
start_listener reply --reply '{"status":"ok"}'
set -l empty_shim (make_shim none)
set -gx PATH $empty_shim
set -e __gpy_nc_probed
__gpy_socket_ready $tmp/agent.sock
set -l no_client $status
set -gx PATH $real_path
test_stop_pids $test_stub_pid
check "with no client on PATH an existing socket file counts as ready" 0 $no_client

finish
test_finish "Socket readiness without timeout"
