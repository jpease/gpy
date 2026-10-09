#!/usr/bin/env bash
# tests/bash/agent_disabled_prompt.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #841: with GPY_AGENT_ENABLED=0 the Bash integration must still install its
# prompt hook and render PS1 through `gpy-agent oneshot`, while starting no
# agent, no supervisor, and no registration. Before the fix __gpy_init
# returned before __gpy_setup_hooks, so PS1 was never set at all.
#
# A stub `gpy-agent` on PATH records every invocation; a live-looking socket
# file proves nothing talks to a daemon even when one could be reached.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

FAILED=0
check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

TMP_DIR="$(mktemp -d)"
mkdir -p "$TMP_DIR/bin" "$TMP_DIR/cache"
CALL_LOG="$TMP_DIR/calls"
: >"$CALL_LOG"

cat >"$TMP_DIR/bin/gpy-agent" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$1" >>"$CALL_LOG"
if [[ "\$1" == "oneshot" ]]; then
    printf 'STUBBED-ONESHOT-%s' "\$2"
fi
exit 0
STUB
chmod +x "$TMP_DIR/bin/gpy-agent"

export PATH="$TMP_DIR/bin:$PATH"
export GPY_AGENT_ENABLED=0
export GPY_AGENT_SUPERVISOR_ENABLED=1
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache"
unset PROMPT_COMMAND

source bash/gpy.bash

check "prompt hook installed with the agent disabled" "yes" \
    "$([[ "${PROMPT_COMMAND:-}" == *__gpy_precmd* ]] && echo yes || echo no)"

mkdir -p "$TMP_DIR/work"
cd "$TMP_DIR/work" || exit 1
__enabled_segments="directory"
__gpy_precmd

case "${PS1:-}" in
    *STUBBED-ONESHOT*) check "PS1 rendered through oneshot" yes yes ;;
    *) check "PS1 rendered through oneshot" yes "no (PS1=[${PS1:-}])" ;;
esac

check "client not registered" "" "${__gpy_registered:-}"

# Renders and theme export may reach the binary; start/status/restart may not.
other_calls="$(grep -Ex 'start|status|restart|stop|register|ping' "$CALL_LOG" | tr '\n' ' ')"
check "agent never started, probed, or restarted" "" "${other_calls% }"

# A supervisor restart check must stay a no-op too (supervisor flag is 1 here).
: >"$CALL_LOG"
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check
check "supervisor check is a no-op" "0" "$(wc -l <"$CALL_LOG" | tr -d ' ')"

cd "$ROOT" || true
rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== agent-disabled prompt test passed ==="
