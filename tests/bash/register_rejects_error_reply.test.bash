#!/usr/bin/env bash
# tests/bash/register_rejects_error_reply.test.bash
# __gpy_register_with_agent succeeds only on a "status":"ok" reply; an {"error":...}
# reply must keep retrying. A stub listener plays the agent.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"

# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

# run_case <name> <reply> <expected-rc> <expected-registered>
run_case() {
    local name="$1" reply="$2" want_rc="$3" want_reg="$4"
    local sock="$test_tmp_dir/$name.sock"
    shell_e2e_stub_listener "$sock" reply --reply "$reply"

    local out
    out="$(
        export GPY_AGENT_SOCKET_PATH="$sock"
        # shellcheck source=bash/core/constants.bash
        source bash/core/constants.bash
        # shellcheck source=bash/core/ipc.bash
        source bash/core/ipc.bash
        # shellcheck source=bash/core/init.bash
        source bash/core/init.bash >/dev/null 2>&1
        __gpy_track_shell_for_agent_recovery() { :; }
        __gpy_registered=""
        __gpy_register_with_agent
        echo "rc=$? registered=[$__gpy_registered]"
    )"
    shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

    if [[ "$out" == *"rc=$want_rc registered=[$want_reg]" ]]; then
        pass "$name"
    else
        fail "$name: want rc=$want_rc registered=[$want_reg], got '$out'"
    fi
}

run_case error_reply '{"error":"x"}' 1 ""
run_case ok_reply '{"status":"ok"}' 0 1

test_finish
