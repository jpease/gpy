#!/usr/bin/env fish
# Fish probes `nc -h` for `-U` before using nc as the IPC client, and Bash and
# Zsh now probe the same way (#850; tests/bash and tests/zsh carry the twins).
# netcat-traditional (the `nc` on some minimal Debian and Ubuntu images) has no
# Unix-socket support, so it must read as "no client" instead of a client that
# fails every request.
#
# `nc` here is a stub script on a PATH that holds nothing else (no socat), so
# the nc branch is the only transport. It logs every invocation.
#   - help text without -U  -> no client: the send fails (status 1) and nc is
#     only ever run as `nc -h`, never with a socket;
#   - help text with -U     -> nc is the client, run as `nc -U <socket> ...`,
#     and its reply comes back.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC nc capability probe (#850)"

if not command -q python3
    test_skip "python3 not installed, cannot create a Unix socket node"
end

set -g tmp (mktemp -d)
set -g real_path $PATH
set -g failures 0
set -g nc_log $tmp/nc.log

function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        print_test_result "$label" FAIL "$detail"
        set -g failures (math $failures + 1)
    end
end

# A socket node nothing listens on: the stub nc never connects to it, but the
# pre-flight `-S` test needs the real thing.
python3 -c 'import socket, sys; s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()' $tmp/agent.sock
if not test -S $tmp/agent.sock
    rm -rf $tmp
    test_skip "could not create a Unix socket node"
end

mkdir -p $tmp/bin
# make_nc HELP_TEXT: a stub nc whose `-h` prints HELP_TEXT and whose other
# invocations answer with a ping reply.
function make_nc --argument-names help
    printf '%s\n' '#!/bin/sh' \
        'echo "$*" >> "$GPY_TEST_NC_LOG"' \
        'if [ "$1" = "-h" ]; then' \
        "    echo '$help'" \
        '    exit 1' \
        fi \
        'cat >/dev/null' \
        "echo '{\"status\":\"ok\"}'" >$tmp/bin/nc
    chmod +x $tmp/bin/nc
    : >$nc_log
end

# run_send: __gpy_ipc_send with only the stub on PATH and a fresh capability
# cache; leaves its status and output in send_status / send_out.
function run_send
    set -e __gpy_nc_probed
    set -gx GPY_TEST_NC_LOG $nc_log
    set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock
    set -g GPY_IPC_TIMEOUT_MS 300
    set -gx PATH $tmp/bin
    set -g send_out (__gpy_ipc_send '{"op":"ping"}' 2>/dev/null)
    set -g send_status $status
    set -gx PATH $real_path
end

make_nc 'usage: nc [-options] hostname port[s] [ports] ... -u UDP mode'
run_send
check "nc without -U is no client: status 1, nothing printed" (test "$send_status" = 1 -a -z "$send_out"; and echo 1; or echo 0) "status $send_status, output [$send_out]"
check "nc without -U is only ever probed with -h, never sent a request" (test "$(cat $nc_log)" = -h; and echo 1; or echo 0) "invoked as: "(string join '|' -- (cat $nc_log))

make_nc 'usage: nc [-U] [-w timeout] ... -U  Use UNIX domain socket'
run_send
check "nc with -U is the client and its reply is returned" (test "$send_status" = 0 -a "$send_out" = '{"status":"ok"}'; and echo 1; or echo 0) "status $send_status, output [$send_out]"
check "nc with -U is run against the agent socket" (string match -q -- "*-U $tmp/agent.sock*" (cat $nc_log | string collect); and echo 1; or echo 0) "invoked as: "(string join '|' -- (cat $nc_log))

rm -rf $tmp
if test $failures -gt 0
    print_test_footer "IPC nc capability probe" FAIL
    exit 1
end
print_test_footer "IPC nc capability probe" PASS
