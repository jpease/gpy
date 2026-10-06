#!/usr/bin/env bash
# tests/bash/ipc_timeout_honors_budget.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #757: the socat transport ran `socat -t 0.1`, a hard
# 100 ms cap that ignored GPY_IPC_TIMEOUT_MS, so a reply arriving later was
# dropped and __gpy_request forked a blocking `gpy-agent oneshot`. (The nc
# branch already honours the budget, so socat is required to exercise the bug.)
#
# A python3 listener replies after 120 ms with GPY_IPC_TIMEOUT_MS=1000; a stub
# `gpy-agent` on PATH logs oneshot calls. The reply must be used and no
# oneshot forked.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command socat "socat not installed; the nc transport already honours the budget"
test_require_command python3 "python3 not installed, cannot simulate a slow-to-reply agent"

# shellcheck source=bash/core/constants.bash
. bash/core/constants.bash
# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-budget.XXXXXX")"
mkdir -p "$tmp/bin" "$tmp/cache"
listener_pid=""
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    [ -n "$listener_pid" ] && kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$tmp"
}
trap cleanup EXIT

oneshot_log="$tmp/oneshot.log"
# shellcheck disable=SC2016 # $ is for the stub script, not this shell
printf '%s\n' '#!/bin/sh' 'echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"' 'echo ONESHOT_OUTPUT' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"

sock="$tmp/slow.sock"
python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1]); s.listen(4); s.settimeout(3)
try:
    while True:
        c, _ = s.accept(); c.recv(65536); time.sleep(float(sys.argv[2]))
        try: c.sendall(b"GITSEGMENT\n")
        except OSError: pass
        c.close()
except OSError:
    pass
' "$sock" 0.12 &
listener_pid=$!
for _ in $(seq 1 30); do
    [ -S "$sock" ] && break
    sleep 0.1
done
[ -S "$sock" ] || test_skip "could not create test Unix socket listener"

: >"$oneshot_log"
out="$(
    export GPY_AGENT_SOCKET_PATH="$sock" GPY_IPC_TIMEOUT_MS=1000 \
        GPY_TEST_ONESHOT_LOG="$oneshot_log" XDG_CACHE_HOME="$tmp/cache" \
        PATH="$tmp/bin:$PATH"
    hash -r
    __gpy_request git "$tmp" ansi true
)"

rc=0
if [ "$out" = "GITSEGMENT" ] && [ ! -s "$oneshot_log" ]; then
    echo "PASS: reply at 120ms within a 1000ms budget is used, no oneshot"
else
    echo "FAIL: output '$out' (want 'GITSEGMENT'), oneshot log: $(cat "$oneshot_log")"
    rc=1
fi
exit "$rc"
