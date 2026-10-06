#!/usr/bin/env zsh
# tests/zsh/ipc_timeout_honors_budget.test.zsh
#
# Regression test for #757: the zsocket and socat transports capped the reply
# wait at 100 ms regardless of GPY_IPC_TIMEOUT_MS, and a connected-but-slow
# agent was treated like an unreachable one, so __gpy_request forked a
# blocking `gpy-agent oneshot` while the agent computed the same segment.
#
# A python3 listener replies after a fixed delay; a stub `gpy-agent` on PATH
# logs every oneshot call.
#   1. reply at 120 ms, budget 1000 ms -> reply used, no oneshot.
#   2. reply at 400 ms, budget 150 ms  -> empty output, no oneshot.
#   3. no socket listening             -> oneshot runs (unreachable agent).

ROOT=${0:a:h:h:h}
# Shared skip contract (#650): exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source zsh/core/constants.zsh
source zsh/core/ipc.zsh

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate a slow-to-reply agent"
fi

tmp=$(mktemp -d)
mkdir -p "$tmp/bin" "$tmp/cache"
listener_pid=""
cleanup() {
    [[ -n "$listener_pid" ]] && kill -9 "$listener_pid" 2>/dev/null
    rm -rf "$tmp"
}
trap cleanup EXIT

oneshot_log="$tmp/oneshot.log"
# The stub reads its log path from the environment, not from a baked-in string.
print -r -- '#!/bin/sh
echo "oneshot $*" >> "$GPY_TEST_ONESHOT_LOG"
echo ONESHOT_OUTPUT' > "$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"

test_result=0
sock="$tmp/slow.sock"

# start_listener DELAY: replies GITSEGMENT after DELAY seconds, bounded life.
start_listener() {
    rm -f "$sock"
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
' "$sock" "$1" &
    listener_pid=$!
    for _ in {1..30}; do
        [[ -S "$sock" ]] && break
        sleep 0.1
    done
    [[ -S "$sock" ]] || test_skip "could not create test Unix socket listener"
}

stop_listener() {
    kill -9 "$listener_pid" 2>/dev/null
    wait "$listener_pid" 2>/dev/null
    listener_pid=""
}

# run_case NAME SOCKET_PATH DELAY BUDGET_MS EXPECT_OUT EXPECT_ONESHOT(0|1)
run_case() {
    local name=$1 socket=$2 delay=$3 budget=$4 expect_out=$5 expect_oneshot=$6
    local out oneshots=0
    [[ -n "$delay" ]] && start_listener "$delay"
    : > "$oneshot_log"
    out=$(
        export GPY_AGENT_SOCKET_PATH="$socket" GPY_IPC_TIMEOUT_MS="$budget" \
            GPY_TEST_ONESHOT_LOG="$oneshot_log" XDG_CACHE_HOME="$tmp/cache" \
            PATH="$tmp/bin:$PATH"
        rehash
        __gpy_request git "$tmp" ansi true
    )
    [[ -n "$delay" ]] && stop_listener
    [[ -s "$oneshot_log" ]] && oneshots=1
    if [[ "$out" == "$expect_out" && "$oneshots" == "$expect_oneshot" ]]; then
        echo "PASS: $name"
    else
        echo "FAIL: $name: output '$out' (want '$expect_out'), oneshot forked=$oneshots (want $expect_oneshot)"
        test_result=1
    fi
}

run_case "reply at 120ms within a 1000ms budget is used, no oneshot" "$sock" 0.12 1000 GITSEGMENT 0
run_case "reply at 400ms past a 150ms budget omits the segment, no oneshot" "$sock" 0.4 150 "" 0
run_case "unreachable socket still falls back to oneshot" "$tmp/missing.sock" "" 150 ONESHOT_OUTPUT 1

exit "$test_result"
