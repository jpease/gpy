#!/usr/bin/env bash
# tests/bash/socat_address_escaping.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A socket path holding socat address metacharacters (`, : ! \ ' "`) still
# reaches the agent through socat, in all three shells. One file drives the
# shells: the behaviour is identical and the listening socket is the costly part.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

command -v socat >/dev/null 2>&1 || test_skip "socat is not installed"
command -v zsh >/dev/null 2>&1 || test_skip "zsh not installed"
command -v fish >/dev/null 2>&1 || test_skip "fish not installed"
command -v python3 >/dev/null 2>&1 || test_skip "python3 is not installed"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-sc.XXXXXX")" || exit 1
SERVER_PID=""
cleanup() {
    [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null
    rm -rf "$SANDBOX"
}
trap cleanup EXIT

# Answers every connection with one JSON line, once per accept.
SERVER="$SANDBOX/server.py"
cat >"$SERVER" <<'PY'
import os, socket, sys
path = sys.argv[1]
srv = socket.socket(socket.AF_UNIX)
srv.bind(path)
srv.listen(8)
print("ready", flush=True)
while True:
    conn, _ = srv.accept()
    conn.recv(4096)
    conn.sendall(b'{"status":"ok"}\n')
    conn.close()
PY

REQUEST='{"op":"ping"}'
n=0
for dir in 'a,b' 'c:d' 'x!!y' 'x\y' "x'y" 'x"y'; do
    n=$((n + 1))
    # A short parent keeps the path under the sockaddr_un length limit.
    mkdir -p "$SANDBOX/$n/$dir"
    sock="$SANDBOX/$n/$dir/g.sock"
    python3 "$SERVER" "$sock" >"$SANDBOX/ready" &
    SERVER_PID=$!
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        [[ -S "$sock" ]] && break
        sleep 0.2
    done

    GPY_AGENT_SOCKET_PATH="$sock" bash -c 'source "$1/bash/core/ipc.bash"; __gpy_send_json "$2" "" out; rc=$?; printf "%s\n%s" "$rc" "$out"' _ "$ROOT" "$REQUEST" >"$SANDBOX/bash.out" 2>&1
    GPY_AGENT_SOCKET_PATH="$sock" zsh -f -c 'source "$1/zsh/core/ipc.zsh"
        # zsocket would connect first and skip socat: hide the module so the socat branch runs.
        function zmodload() { [[ $1 == zsh/net/socket ]] && return 1; builtin zmodload "$@"; }
        __gpy_send_json "$2"; print -r -- "rc=$?"' _ "$ROOT" "$REQUEST" >"$SANDBOX/zsh.out" 2>&1
    # shellcheck disable=SC2016 # the child fish expands its own variables
    GPY_AGENT_SOCKET_PATH="$sock" fish --no-config -c 'source "$argv[1]/fish/core/constants.fish"; source "$argv[1]/fish/core/util.fish"; source "$argv[1]/fish/core/ipc.fish"; __gpy_ipc_send $argv[2]; echo "rc=$status"' "$ROOT" "$REQUEST" >"$SANDBOX/fish.out" 2>&1

    # shellcheck disable=SC2016 # literal text for the assertion label
    label="path dir [$dir]"
    if [[ "$(head -n1 "$SANDBOX/bash.out")" == 0 ]] && grep -q '"status":"ok"' "$SANDBOX/bash.out"; then
        pass "bash reaches an agent through socat: $label"
    else
        fail "bash: $label: $(cat "$SANDBOX/bash.out")"
    fi
    if grep -q '"status":"ok"' "$SANDBOX/zsh.out" && grep -q '^rc=0$' "$SANDBOX/zsh.out"; then
        pass "zsh reaches an agent through socat: $label"
    else
        fail "zsh: $label: $(cat "$SANDBOX/zsh.out")"
    fi
    if grep -q '"status":"ok"' "$SANDBOX/fish.out" && grep -q '^rc=0$' "$SANDBOX/fish.out"; then
        pass "fish reaches an agent through socat: $label"
    else
        fail "fish: $label: $(cat "$SANDBOX/fish.out")"
    fi
    kill "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
    SERVER_PID=""
done

test_finish "socat reaches sockets under metacharacter paths"
