#!/usr/bin/env bash
# tests/bash/shell_e2e_stop_agent_term.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# When `gpy-agent stop` has not released the socket, shell_e2e_stop_agent must
# not go straight to SIGKILL on whatever holds it: an agent that would exit
# cleanly on TERM must get to remove its socket and finish a cache write. It
# follows the same TERM -> bounded wait -> KILL sequence as
# shell_e2e_stop_agent_under.
#
# A python3 stub holds an AF_UNIX socket, ignores the `stop` request (the
# stub "binary" does nothing) and records TERM in a file from its handler
# before exiting 0. The test asserts the record exists: a straight KILL
# leaves no record. A second stub that ignores TERM must still be KILLed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

for _tool in python3 lsof; do
    command -v "$_tool" >/dev/null 2>&1 || test_skip "$_tool not available"
done

scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpy-stop-term.XXXXXX")" || exit 1
stub_pid=""
cleanup() {
    [ -n "$stub_pid" ] && kill -9 "$stub_pid" 2>/dev/null
    rm -rf "$scratch"
}
trap cleanup EXIT

# Fake agent CLI: `stop` succeeds but releases nothing.
printf '#!/bin/sh\nexit 0\n' >"$scratch/fake-agent"
chmod +x "$scratch/fake-agent"
SHELL_E2E_AGENT_BIN="$scratch/fake-agent"
GPY_AGENT_SOCKET_PATH="$scratch/gpy.sock"

# $1 = "term" (record TERM, exit 0) or "ignore" (ignore TERM, hold on).
start_holder() {
    rm -f "$GPY_AGENT_SOCKET_PATH" "$scratch/record" "$scratch/ready"
    python3 - "$GPY_AGENT_SOCKET_PATH" "$scratch/record" "$scratch/ready" "$1" <<'PY' &
import os, signal, socket, sys, time
path, record, ready, mode = sys.argv[1:5]
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
def on_term(*_):
    with open(record, "w") as f:
        f.write("TERM\n")
    os.unlink(path)
    os._exit(0)
signal.signal(signal.SIGTERM, on_term if mode == "term" else signal.SIG_IGN)
open(ready, "w").close()
while True:
    time.sleep(0.1)
PY
    stub_pid=$!
    disown "$stub_pid"
    shell_e2e_poll 5 test -e "$scratch/ready"
}


if start_holder term; then
    shell_e2e_stop_agent
    if [ "$(cat "$scratch/record" 2>/dev/null)" = "TERM" ]; then
        pass "a holder that exits cleanly on TERM receives TERM"
    else
        fail "the holder was killed without a TERM step"
    fi
else
    fail "stub holder did not start"
fi

if start_holder ignore; then
    shell_e2e_stop_agent
    if shell_e2e_path_unbound "$GPY_AGENT_SOCKET_PATH" && ! kill -0 "$stub_pid" 2>/dev/null; then
        pass "a holder that ignores TERM is still killed"
    else
        fail "a holder that ignores TERM survived shell_e2e_stop_agent"
    fi
else
    fail "stub holder did not start"
fi

test_finish "shell_e2e_stop_agent escalates TERM then KILL"
