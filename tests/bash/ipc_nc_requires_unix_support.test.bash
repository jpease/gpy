#!/usr/bin/env bash
# tests/bash/ipc_nc_requires_unix_support.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash probes `nc -h` for `-U` before using nc (as Fish does); netcat-traditional
# lacks Unix sockets, so without -U nc is only ever run as `nc -h`.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command python3 "python3 not installed, cannot create a Unix socket node"

# shellcheck source=bash/core/constants.bash
. bash/core/constants.bash
# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-nc-probe.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

# The stub nc never connects, but the pre-flight `-S` test needs a real socket.
python3 -c 'import socket, sys; s = socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()' "$tmp/agent.sock"
[ -S "$tmp/agent.sock" ] || test_skip "could not create a Unix socket node"

mkdir -p "$tmp/bin"
log="$tmp/nc.log"
make_nc() {
    cat >"$tmp/bin/nc" <<EOF
#!/bin/sh
echo "\$*" >> "$log"
if [ "\$1" = "-h" ]; then
    echo "$1"
    exit 1
fi
cat >/dev/null
echo '{"status":"ok"}'
EOF
    chmod +x "$tmp/bin/nc"
    : >"$log"
}

# Leaves status and output in send_status / send_out.
run_send() {
    local saved_path="$PATH"
    __gpy_nc_supports_unix=""
    export GPY_AGENT_SOCKET_PATH="$tmp/agent.sock" GPY_IPC_TIMEOUT_MS=300
    PATH="$tmp/bin"
    send_out="$(__gpy_send_json '{"op":"ping"}')"
    send_status=$?
    PATH="$saved_path"
}

make_nc "usage: nc [-options] hostname port[s] [ports] ... -u UDP mode"
run_send
if [ "$send_status" = 1 ] && [ -z "$send_out" ]; then
    pass "nc without -U is no client: status 1, nothing printed"
else
    fail "nc without -U: status $send_status, output [$send_out] (want status 1, empty output)"
fi
if [ "$(cat "$log")" = "-h" ]; then
    pass "nc without -U is only ever probed with -h, never sent a request"
else
    fail "nc without -U was invoked as: $(tr '\n' '|' <"$log")"
fi

make_nc "usage: nc [-U] [-w timeout] ... -U  Use UNIX domain socket"
run_send
if [ "$send_status" = 0 ] && [ "$send_out" = '{"status":"ok"}' ]; then
    pass "nc with -U is the client and its reply is returned"
else
    fail "nc with -U: status $send_status, output [$send_out] (want status 0 and the ping reply)"
fi
if grep -q -- "-U $tmp/agent.sock" "$log"; then
    pass "nc with -U is run against the agent socket"
else
    fail "nc with -U never got the socket: $(tr '\n' '|' <"$log")"
fi

test_finish "bash nc capability probe"
