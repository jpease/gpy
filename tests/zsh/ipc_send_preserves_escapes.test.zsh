#!/usr/bin/env zsh
# tests/zsh/ipc_send_preserves_escapes.test.zsh
#
# zsh's `print` (without -r) and `echo` interpret backslash escapes, so the
# send path must not use them: the doubled backslashes from __gpy_escape_json_into
# would collapse (`a\b` would reach the agent as `a<BS>b`, `notes\stuff` as
# invalid JSON), and every reply, cache key and prompt string would lose its
# `\c`/`\b` sequences.
#
# A stub listener records the exact request line it receives and replies
# with literal backslash sequences. The test asserts that the request
# decodes to the original value, that the reply comes back byte for byte,
# and that __gpy_find_git_root returns a repo root containing `\` verbatim.

ROOT=${0:a:h:h:h}
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
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# --- Request and reply round-trip through __gpy_send_json ---

sock="$test_tmp_dir/gpy-escape-test.sock"
recorded="$test_tmp_dir/request.line"
expected_reply='dir a\b \c tail'
shell_e2e_stub_listener "$sock" reply --reply "$expected_reply" --requests "$recorded"

export GPY_AGENT_SOCKET_PATH="$sock"
d='/x/a\b"q'
__gpy_escape_json_into escaped_d "$d"
reply="$(__gpy_send_json "{\"cwd\":\"$escaped_d\"}")"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

if python3 -c '
import json, sys
line = open(sys.argv[1], "rb").read()
sys.exit(0 if json.loads(line)["cwd"] == sys.argv[2] else 1)
' "$recorded" "$d" 2>/dev/null; then
    pass "request JSON decodes to the original cwd"
else
    fail "request JSON decodes to the original cwd: sent $(od -c "$recorded" 2>/dev/null | head -3)"
fi

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
actual_root=""
__gpy_find_git_root "$expected_root" actual_root
if [[ "$actual_root" == "$expected_root" ]]; then
    pass "__gpy_find_git_root returns the byte-exact root"
else
    fail "__gpy_find_git_root returns the byte-exact root: got $(print -r -- "$actual_root" | od -c | tail -3)"
fi

test_finish "zsh IPC send and reply preserve backslashes"
