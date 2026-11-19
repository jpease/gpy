#!/usr/bin/env bash
# tests/bash/agent_disabled_prompt.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# With GPY_AGENT_ENABLED=0 Bash still installs its prompt hook and `gpy
# prompt` still draws PS1, but no agent is ever started, not even by the
# supervisor (enabled here and due on every prompt).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

GPY_BIN="$ROOT/gpy-agent/target/debug/gpy"
[[ -x "$GPY_BIN" ]] || test_skip "gpy is not built (cargo build --bins)"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/bin" "$TMP_DIR/config/gpy" "$TMP_DIR/cache" "$TMP_DIR/runtime" "$TMP_DIR/work"
chmod 700 "$TMP_DIR/runtime"
CALL_LOG="$TMP_DIR/calls"
: >"$CALL_LOG"

# gpy starts the gpy-agent beside it: a stub that records every call.
cp "$GPY_BIN" "$TMP_DIR/bin/gpy"
cat >"$TMP_DIR/bin/gpy-agent" <<STUB
#!/bin/sh
echo "\$*" >>"$CALL_LOG"
exit 0
STUB
chmod +x "$TMP_DIR/bin/gpy-agent"

# shellcheck disable=SC2016 # the script expands in the child shell
out="$(cd "$TMP_DIR/work" && env -i HOME="$TMP_DIR" PATH="$TMP_DIR/bin:/usr/bin:/bin" \
    XDG_CONFIG_HOME="$TMP_DIR/config" XDG_CACHE_HOME="$TMP_DIR/cache" \
    XDG_RUNTIME_DIR="$TMP_DIR/runtime" GPY_AGENT_SOCKET_PATH="$TMP_DIR/agent.sock" \
    GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0 GPY_NERD_FONT=none \
    "$BASH" --norc --noprofile -c '
        source "$0/bash/gpy.bash"
        [[ $PROMPT_COMMAND == *__gpy_precmd* ]] && echo hook=yes
        __gpy_precmd; __gpy_precmd; __gpy_precmd
        printf "PS1=%s\n" "$PS1"
    ' "$ROOT" 2>&1)"

check "prompt hook installed with the agent disabled" "hook=yes" "$(grep '^hook=' <<<"$out")"
# PS1 may span lines (a leading newline): everything after the marker.
ps1="${out#*PS1=}"
case "$ps1" in
    *work*) pass "gpy prompt drew PS1 (directory segment present)" ;;
    *) fail "gpy prompt drew PS1: got [$ps1]" ;;
esac
# Absence cannot be polled for: give a detached start a fixed window.
sleep 0.5
check "agent never started, not at the first prompt nor by the supervisor" "" "$(tr '\n' ' ' <"$CALL_LOG")"

test_finish "agent-disabled prompt test"
