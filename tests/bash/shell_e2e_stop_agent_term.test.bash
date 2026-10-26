#!/usr/bin/env bash
# tests/bash/shell_e2e_stop_agent_term.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shell_e2e_stop_agent must TERM, wait, then KILL an agent that kept its socket
# after `stop`. A stub listener records TERM; a second one ignores TERM and must be KILLed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

for _tool in python3 lsof; do
    command -v "$_tool" >/dev/null 2>&1 || test_skip "$_tool not available"
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpy-stop-term.XXXXXX")" || exit 1
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$scratch"
}
trap cleanup EXIT

# Fake agent CLI: `stop` succeeds but releases nothing.
printf '#!/bin/sh\nexit 0\n' >"$scratch/fake-agent"
chmod +x "$scratch/fake-agent"
SHELL_E2E_AGENT_BIN="$scratch/fake-agent"
GPY_AGENT_SOCKET_PATH="$scratch/gpy.sock"

shell_e2e_stub_listener "$GPY_AGENT_SOCKET_PATH" silent --term-log "$scratch/record"
shell_e2e_stop_agent
if [ "$(cat "$scratch/record" 2>/dev/null)" = "TERM" ]; then
    pass "a holder that exits cleanly on TERM receives TERM"
else
    fail "the holder was killed without a TERM step"
fi

shell_e2e_stub_listener "$GPY_AGENT_SOCKET_PATH" silent --ignore-term
shell_e2e_stop_agent
if shell_e2e_path_unbound "$GPY_AGENT_SOCKET_PATH" && ! kill -0 "$SHELL_E2E_STUB_PID" 2>/dev/null; then
    pass "a holder that ignores TERM is still killed"
else
    fail "a holder that ignores TERM survived shell_e2e_stop_agent"
fi

test_finish "shell_e2e_stop_agent escalates TERM then KILL"
