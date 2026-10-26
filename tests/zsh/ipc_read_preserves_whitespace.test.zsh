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
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

sock="$test_tmp_dir/gpy-whitespace-test.sock"
reply='  ❯ \D{%H} '
shell_e2e_stub_listener "$sock" reply --reply "$reply"

export GPY_AGENT_SOCKET_PATH="$sock"
result="$(__gpy_send_json '{"op":"ping"}'; print -n x)"
result="${result%x}"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

if [[ "$result" == "$reply"$'\n' ]]; then
    pass "zsocket read preserves leading/trailing whitespace and backslashes"
else
    fail "zsocket read preserves leading/trailing whitespace and backslashes: got '${(q+)result}'"
fi

test_finish
