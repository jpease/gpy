#!/usr/bin/env zsh
# tests/zsh/ipc_nc_requires_unix_support.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh probes `nc -h` for `-U` before using nc as the IPC client, exactly as
# Fish does. netcat-traditional (the `nc` on some minimal Debian and
# Ubuntu images) has no Unix-socket support, so running `nc -U` against it
# would fail on every request.
#
# `nc` here is a stub script on a PATH that holds nothing else (no socat), and
# `zmodload` is shadowed so zsocket is unavailable: nc is the only transport.
# The stub logs every invocation.
#   - help text without -U  -> no client: the send fails (status 1) and nc is
#     only ever run as `nc -h`, never with a socket;
#   - help text with -U     -> nc is the client, run as `nc -U <socket> ...`,
#     and its reply comes back.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

test_require_command python3 "python3 not installed, cannot create a Unix socket node"

source zsh/core/constants.zsh
source zsh/core/ipc.zsh

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-nc-probe.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

zmodload() { [[ "$1" == "zsh/net/socket" ]] && return 1; builtin zmodload "$@"; }

# A socket node nothing listens on: the stub nc never connects to it, but the
# pre-flight `-S` test needs the real thing.
shell_e2e_stub_listener "$tmp/agent.sock" bind-only

mkdir -p "$tmp/bin"
log="$tmp/nc.log"
# make_nc HELP_TEXT: a stub nc whose `-h` prints HELP_TEXT and whose other
# invocations answer with a ping reply.
make_nc() {
    cat >"$tmp/bin/nc" <<STUB
#!/bin/sh
echo "\$*" >> "$log"
if [ "\$1" = "-h" ]; then
    echo "$1"
    exit 1
fi
cat >/dev/null
echo '{"status":"ok"}'
STUB
    chmod +x "$tmp/bin/nc"
    : >"$log"
}

# run_send: __gpy_send_json with only the stub on PATH, in a fresh capability
# cache; leaves its status and output in send_status / send_out.
run_send() {
    local saved_path=$PATH
    __gpy_nc_supports_unix="" __gpy_nc_halfclose_flag=""
    export GPY_AGENT_SOCKET_PATH="$tmp/agent.sock" GPY_IPC_TIMEOUT_MS=300
    PATH="$tmp/bin"
    rehash
    send_out=$(__gpy_send_json '{"op":"ping"}')
    send_status=$?
    PATH=$saved_path
    rehash
}

make_nc "usage: nc [-options] hostname port[s] [ports] ... -u UDP mode"
run_send
if [[ "$send_status" == 1 && -z "$send_out" ]]; then
    pass "nc without -U is no client: status 1, nothing printed"
else
    fail "nc without -U: status $send_status, output [$send_out] (want status 1, empty output)"
fi
if [[ "$(cat "$log")" == "-h" ]]; then
    pass "nc without -U is only ever probed with -h, never sent a request"
else
    fail "nc without -U was invoked as: $(tr '\n' '|' <"$log")"
fi

make_nc "usage: nc [-U] [-w timeout] ... -U  Use UNIX domain socket"
run_send
if [[ "$send_status" == 0 && "$send_out" == '{"status":"ok"}' ]]; then
    pass "nc with -U is the client and its reply is returned"
else
    fail "nc with -U: status $send_status, output [$send_out] (want status 0 and the ping reply)"
fi
if grep -q -- "-U $tmp/agent.sock" "$log"; then
    pass "nc with -U is run against the agent socket"
else
    fail "nc with -U never got the socket: $(tr '\n' '|' <"$log")"
fi

# OpenBSD nc (Debian/Ubuntu) half-closes only with -N, so the agent sees EOF;
# its help line reads "Shutdown the network socket after EOF on stdin".
make_nc "$(printf 'usage: nc [-46CDdFhklNnrStUuvZz]\n\t-N\t\tShutdown the network socket after EOF on stdin\n\t-U\t\tUse UNIX domain socket')"
run_send
if grep -q -- "-U $tmp/agent.sock -w 1 -N" "$log"; then
    pass "OpenBSD nc is run with -N so it half-closes"
else
    fail "OpenBSD nc was not given -N: $(tr '\n' '|' <"$log")"
fi

# Apple's nc half-closes by itself; its -N takes an argument (num_probes).
make_nc "$(printf 'usage: nc [-46AacCDdEFhklMnOortUuvz]\n\t-N num_probes                 Number of probes to send\n\t-U                            Use UNIX domain socket')"
run_send
if grep -q -- "-U $tmp/agent.sock" "$log" && ! grep -q -- " -N" "$log"; then
    pass "Apple-style nc (-N num_probes) is never given -N"
else
    fail "Apple-style nc was invoked as: $(tr '\n' '|' <"$log")"
fi

test_finish "zsh nc capability probe"
