#!/usr/bin/env bash
# tests/bash/ipc_timeout_honors_budget.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# socat must honour GPY_IPC_TIMEOUT_MS, not a hard 100 ms cap: a 120 ms reply
# under a 1000 ms budget is used and no oneshot forked. Needs socat.

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
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT

oneshot_log="$tmp/oneshot.log"
# shellcheck disable=SC2016 # $ is for the stub script, not this shell
printf '%s\n' '#!/bin/sh' 'echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"' 'echo ONESHOT_OUTPUT' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"

sock="$tmp/slow.sock"
shell_e2e_stub_listener "$sock" reply --reply GITSEGMENT --delay 0.12

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
