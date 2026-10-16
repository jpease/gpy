#!/usr/bin/env zsh
# tests/zsh/ipc_read_preserves_whitespace.test.zsh
#
# The zsocket reply read must keep leading/trailing whitespace and backslashes:
# replies are rendered prompt fragments (the character segment ends in a space,
# the clock embeds `\D{...}`), and a bare `read` strips or mangles both.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! zmodload zsh/net/socket 2>/dev/null; then
    test_skip "zsh/net/socket unavailable, zsocket read path not reachable"
fi

test_tmp_dir=$(mktemp -d)
sock="$test_tmp_dir/gpy-whitespace-test.sock"
reply='  ❯ \D{%H} '

python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.recv(4096)
conn.sendall(sys.argv[2].encode() + b"\n")
conn.close()
' "$sock" "$reply" &
listener_pid=$!
disown 2>/dev/null

if ! shell_e2e_poll 2 test -S "$sock"; then
    kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$test_tmp_dir"
    test_skip "could not create test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock"
result="$(__gpy_send_json '{"op":"ping"}'; print -n x)"
result="${result%x}"

kill -9 "$listener_pid" 2>/dev/null
rm -rf "$test_tmp_dir"

if [[ "$result" == "$reply"$'\n' ]]; then
    pass "zsocket read preserves leading/trailing whitespace and backslashes"
else
    fail "zsocket read preserves leading/trailing whitespace and backslashes: got '${(q+)result}'"
fi

test_finish
