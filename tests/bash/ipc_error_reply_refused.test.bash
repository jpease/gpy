#!/usr/bin/env bash
# tests/bash/ipc_error_reply_refused.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The reply contract: a reply line starting with `{"error"` is a permanent
# refusal. __gpy_send_json returns 3 and never prints it (OUTVAR holds it for
# callers that tell refusals apart); a requester with a oneshot fallback
# takes it, one without renders nothing. An empty reply line still means
# "alive but late" (status 2, no oneshot). A stub listener plays the agent.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command python3 "python3 not installed, cannot simulate an agent"
if ! command -v socat &>/dev/null && ! command -v nc &>/dev/null; then
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
fi

# shellcheck source=bash/core/constants.bash
. bash/core/constants.bash
# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-refused.XXXXXX")"
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/bin" "$tmp/cache" "$tmp/run"
export XDG_RUNTIME_DIR="$tmp/run" XDG_CACHE_HOME="$tmp/cache" GPY_IPC_TIMEOUT_MS=1000
oneshot_log="$tmp/oneshot.log"
# shellcheck disable=SC2016 # $ is for the stub script, not this shell
printf '%s\n' '#!/bin/sh' 'echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"' 'echo ONESHOT_OUTPUT' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"
export GPY_TEST_ONESHOT_LOG="$oneshot_log"
PATH="$tmp/bin:$PATH"
hash -r

error_reply='{"error":"Parse error: Invalid control character"}'
export GPY_AGENT_SOCKET_PATH="$tmp/refusing.sock"
shell_e2e_stub_listener "$GPY_AGENT_SOCKET_PATH" reply --reply "$error_reply"

out="$(__gpy_send_json '{"op":"ping"}')"
check "a refusal is status 3" "3" "$?"
check "a refusal is never printed" "" "$out"

outvar="stale"
__gpy_send_json '{"op":"ping"}' "" outvar
check "a refusal is status 3 with OUTVAR" "3" "$?"
check "OUTVAR holds the refusal" "$error_reply" "$outvar"

: >"$oneshot_log"
out="$(__gpy_request directory "$tmp" bash-prompt true "" true)"
check "a refused directory request takes the oneshot fallback" "ONESHOT_OUTPUT" "$out"
check "the oneshot ran once" "1" "$(shell_e2e_count_lines "$oneshot_log")"

out="$(__gpy_request_exit_status 1 true "" true)"
check "a refused exit_status request renders nothing" "1:" "$?:$out"

out="$(__gpy_request_clock true "" true)"
check "a refused clock request renders nothing" "1:" "$?:$out"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# An empty line is an agent that answered in time with nothing yet: late,
# never a oneshot.
export GPY_AGENT_SOCKET_PATH="$tmp/empty.sock"
shell_e2e_stub_listener "$GPY_AGENT_SOCKET_PATH" reply --reply ''
out="$(__gpy_send_json '{"op":"ping"}')"
check "an empty reply line is status 2" "2" "$?"
: >"$oneshot_log"
out="$(__gpy_request directory "$tmp" bash-prompt true "" true)"
check "an empty directory reply omits the segment" "" "$out"
check "an empty directory reply forks no oneshot" "0" "$(shell_e2e_count_lines "$oneshot_log")"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

test_finish "bash treats an error reply as a refusal and an empty one as late"
