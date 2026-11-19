#!/usr/bin/env zsh
# tests/zsh/agent_disabled_prompt.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With GPY_AGENT_ENABLED=0 the Zsh prompt is still drawn (by `gpy prompt
# zsh`, without an agent) and nothing is started: no agent start on the
# first prompt and no supervisor restart later, even with the supervisor
# enabled and checking on every prompt.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1
test_require_command gpy

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-agent-disabled.XXXXXX") || exit 1
TMP_DIR=${TMP_DIR:A}
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/bin" "$TMP_DIR/cache" "$TMP_DIR/config" "$TMP_DIR/run" "$TMP_DIR/work"
CALL_LOG="$TMP_DIR/calls"
: >"$CALL_LOG"

# The binary starts the gpy-agent beside itself: a copy of gpy next to a
# recording gpy-agent stub.
cp "${commands[gpy]}" "$TMP_DIR/bin/gpy"
cat >"$TMP_DIR/bin/gpy-agent" <<STUB
#!/bin/sh
printf '%s\n' "\$1" >>"$CALL_LOG"
exit 0
STUB
chmod +x "$TMP_DIR/bin/gpy-agent"

path=("$TMP_DIR/bin" $path)
export GPY_AGENT_ENABLED=0
export GPY_AGENT_SUPERVISOR_ENABLED=1
export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache" XDG_CONFIG_HOME="$TMP_DIR/config" XDG_RUNTIME_DIR="$TMP_DIR/run"

source zsh/gpy.zsh

check "precmd hook installed with the agent disabled" yes \
    "$([[ ${precmd_functions[(r)__gpy_precmd]} == __gpy_precmd ]] && echo yes || echo no)"

cd "$TMP_DIR/work"
__gpy_precmd
if [[ -n $PROMPT && $PROMPT != '❯ ' && $PROMPT == *work* ]]; then
    pass "the binary drew the prompt"
else
    fail "the prompt was not drawn by the binary: [$PROMPT]"
fi

# A second prompt: the supervisor would check (and restart) here.
__gpy_precmd
check "agent never started, probed, or restarted" "" "$(tr '\n' ' ' <"$CALL_LOG")"
check "no socket appeared" no "$([[ -e $GPY_AGENT_SOCKET_PATH ]] && echo yes || echo no)"

cd "$ROOT"
test_finish "agent-disabled prompt test"
