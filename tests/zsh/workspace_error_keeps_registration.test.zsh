#!/usr/bin/env zsh
# tests/zsh/workspace_error_keeps_registration.test.zsh
#
# Like fish's, __gpy_sync_workspace must
# keep the registration when the workspace reply is an error other than
# "not registered" (e.g. `cd /etc` rejected by the path validator). Clearing
# __gpy_registered would make the re-register from the same rejected directory
# fail too, and every later cd would skip the sync.
#
# A stub listener plays the agent: `workspace`/`register` from a directory
# named `denied` -> error; `workspace` from `lost` -> "not registered"; else
# ok. Every request is logged. The agent is never started; the listener is
# stopped on exit.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi
if ! (( $+commands[socat] )) && ! (( $+commands[nc] )); then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

test_tmp_dir="$(mktemp -d)"
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$test_tmp_dir"
}
trap cleanup EXIT

sock="$test_tmp_dir/agent.sock"
requests="$test_tmp_dir/requests.log"
mkdir -p "$test_tmp_dir/proj" "$test_tmp_dir/denied" "$test_tmp_dir/lost" "$test_tmp_dir/other"

shell_e2e_stub_listener "$sock" reply --reply '{"status":"ok"}' --requests "$requests" \
    --route '"op":"(workspace|register)".*"cwd":"[^"]*/denied"' \
    '{"error":"Invalid workspace path: Access to system directories denied"}' \
    --route '"op":"workspace".*"cwd":"[^"]*/lost"' \
    '{"error":"PID 123 not registered - please register first"}'

count_ops() { grep -c "\"op\":\"$1\"" "$requests"; }

export GPY_AGENT_SOCKET_PATH="$sock"
source zsh/core/constants.zsh
source zsh/core/ipc.zsh
source zsh/core/init.zsh >/dev/null 2>&1
function __gpy_track_shell_for_agent_recovery() { :; }
: >"$requests"
__gpy_registered=""
__gpy_last_workspace=""

cd "$test_tmp_dir/proj" || exit 1
__gpy_register_with_agent
check "initial register" "1" "$__gpy_registered"

cd "$test_tmp_dir/denied" || exit 1
before_ws="$__gpy_last_workspace"
__gpy_sync_workspace
check "error reply keeps __gpy_registered" "1" "$__gpy_registered"
check "error reply does not re-register" "1" "$(count_ops register)"
check "error reply leaves __gpy_last_workspace untouched" "$before_ws" "$__gpy_last_workspace"

# Further prompts in the rejected directory must not re-send the workspace op.
__gpy_sync_workspace
__gpy_sync_workspace
__gpy_sync_workspace
check "rejected dir is sent exactly once across prompts" "1" "$(count_ops workspace)"

cd "$test_tmp_dir/other" || exit 1
__gpy_sync_workspace
check "next cd sends a workspace op" "2" "$(count_ops workspace)"
check "next cd records __gpy_last_workspace" "$test_tmp_dir/other" "$__gpy_last_workspace"

cd "$test_tmp_dir/denied" || exit 1
__gpy_sync_workspace
check "cd back into rejected dir re-sends" "3" "$(count_ops workspace)"
__gpy_sync_workspace
check "and is again sent only once" "3" "$(count_ops workspace)"

cd "$test_tmp_dir/lost" || exit 1
__gpy_sync_workspace
check "'not registered' re-registers" "2" "$(count_ops register)"
check "re-register restores registration" "1" "$__gpy_registered"

test_finish
