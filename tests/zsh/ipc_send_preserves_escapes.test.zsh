#!/usr/bin/env zsh
# tests/zsh/ipc_send_preserves_escapes.test.zsh
#
# Regression test for #676: zsh's `print` (without -r) and `echo` interpret
# backslash escapes, so the send path turned the doubled backslashes from
# __gpy_escape_json back into single ones (`a\b` reached the agent as
# `a<BS>b`, `notes\stuff` as invalid JSON), and every reply, cache key and
# prompt string re-emitted with `echo` lost its `\c`/`\b` sequences.
#
# A Python listener records the exact request line it receives and replies
# with literal backslash sequences. The test asserts that the request
# decodes to the original value, that the reply comes back byte for byte,
# and that __gpy_find_git_root returns a repo root containing `\` verbatim.

ROOT=${0:a:h:h:h}
# Shared skip contract (#650): exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! command -v git &>/dev/null; then
    test_skip "git not installed, cannot exercise __gpy_find_git_root"
fi

test_tmp_dir=$(mktemp -d)
test_result=0

pass() { print -r -- "PASS: $1"; }
fail() { print -r -- "FAIL: $1"; test_result=1; }

# --- Request and reply round-trip through __gpy_send_json ---

sock="$test_tmp_dir/gpy-escape-test.sock"
recorded="$test_tmp_dir/request.line"
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
s.settimeout(5)
conn, _ = s.accept()
data = b""
while not data.endswith(b"\n"):
    chunk = conn.recv(65536)
    if not chunk:
        break
    data += chunk
open(sys.argv[2], "wb").write(data)
conn.sendall(b"dir a\\b \\c tail\n")
conn.close()
' "$sock" "$recorded" &
listener_pid=$!
disown 2>/dev/null

for _ in {1..20}; do
    [[ -S "$sock" ]] && break
    sleep 0.1
done
if [[ ! -S "$sock" ]]; then
    kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$test_tmp_dir"
    test_skip "could not create test Unix socket listener"
fi

export GPY_AGENT_SOCKET_PATH="$sock"
d='/x/a\b"q'
reply="$(__gpy_send_json "{\"cwd\":\"$(__gpy_escape_json "$d")\"}")"
kill -9 "$listener_pid" 2>/dev/null

if python3 -c '
import json, sys
line = open(sys.argv[1], "rb").read()
sys.exit(0 if json.loads(line)["cwd"] == sys.argv[2] else 1)
' "$recorded" "$d" 2>/dev/null; then
    pass "request JSON decodes to the original cwd"
else
    fail "request JSON decodes to the original cwd: sent $(od -c "$recorded" 2>/dev/null | head -3)"
fi

expected_reply='dir a\b \c tail'
if [[ "$reply" == "$expected_reply" ]]; then
    pass "reply is printed byte for byte"
else
    fail "reply is printed byte for byte: got $(print -r -- "$reply" | od -c | head -2)"
fi

# --- __gpy_find_git_root keeps backslashes in the repo path ---

repo_dir="$test_tmp_dir/a\\b"
mkdir -p "$repo_dir"
git -C "$repo_dir" init -q
expected_root=$(cd "$repo_dir" && pwd -P)
actual_root=$(__gpy_find_git_root "$expected_root")
if [[ "$actual_root" == "$expected_root" ]]; then
    pass "__gpy_find_git_root returns the byte-exact root"
else
    fail "__gpy_find_git_root returns the byte-exact root: got $(print -r -- "$actual_root" | od -c | tail -3)"
fi

rm -rf "$test_tmp_dir"
exit "$test_result"
