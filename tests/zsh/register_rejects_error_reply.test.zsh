#!/usr/bin/env zsh
# tests/zsh/register_rejects_error_reply.test.zsh
#
# __gpy_register_with_agent must count a
# registration as successful only when the reply carries "status":"ok" (as
# fish does). Treating an {"error":...} reply as success would set
# __gpy_registered=1, so the shell would stop retrying and receive no repaint
# doorbells.
#
# A stub listener plays the agent and answers every request (sourcing init
# may itself register once); it is stopped on exit.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! (( $+commands[socat] )) && ! (( $+commands[nc] )); then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir=$(mktemp -d)

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
        source zsh/core/constants.zsh
        source zsh/core/ipc.zsh
        source zsh/core/init.zsh >/dev/null 2>&1
        function __gpy_track_shell_for_agent_recovery() { :; }
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
