#!/usr/bin/env zsh
# tests/zsh/agent_disabled_prompt.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #841: with GPY_AGENT_ENABLED=0 the Zsh integration must render PROMPT
# through `gpy-agent oneshot` and start/register nothing: no agent start, no
# supervisor, no registration. (Zsh used to register unconditionally.)

ROOT=${0:a:h:h:h}
cd "$ROOT"

FAILED=0
check() {
    local label=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

TMP_DIR=$(mktemp -d)
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

source zsh/gpy.zsh

check "precmd hook installed with the agent disabled" yes \
    "$([[ ${precmd_functions[(r)__gpy_precmd]} == __gpy_precmd ]] && echo yes || echo no)"

mkdir -p "$TMP_DIR/work"
cd "$TMP_DIR/work"
__enabled_segments=(directory)
__gpy_precmd

case "${PROMPT:-}" in
    *STUBBED-ONESHOT*) check "PROMPT rendered through oneshot" yes yes ;;
    *) check "PROMPT rendered through oneshot" yes "no (PROMPT=[${PROMPT:-}])" ;;
esac

check "client not registered" "" "${__gpy_registered:-}"

other_calls=$(grep -Ex 'start|status|restart|stop|register|ping' "$CALL_LOG" | tr '\n' ' ')
check "agent never started, probed, or restarted" "" "${other_calls% }"

# A supervisor restart check must stay a no-op too (supervisor flag is 1 here).
: >"$CALL_LOG"
__gpy_supervisor_last_check_time=0
__gpy_supervisor_check
check "supervisor check is a no-op" "0" "$(wc -l <"$CALL_LOG" | tr -d ' ')"

cd "$ROOT"
rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== agent-disabled prompt test passed ==="
