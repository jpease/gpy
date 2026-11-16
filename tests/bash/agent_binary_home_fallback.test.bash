#!/usr/bin/env bash
# tests/bash/agent_binary_home_fallback.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The installer puts gpy-agent in ~/.local/bin, which is not always on PATH.
# As in Fish, the shell then runs that copy for start, theme export and
# oneshot; a gpy-agent on PATH still wins.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
LOG="$TMP_DIR/calls"

# write_stub DIR TAG: a gpy-agent that logs "TAG <argv>" and prints TAG on oneshot.
write_stub() {
    mkdir -p "$1"
    cat >"$1/gpy-agent" <<STUB
#!/bin/sh
echo "$2 \$*" >>"$LOG"
[ "\$1" = oneshot ] && printf '%s' "$2"
exit 0
STUB
    chmod +x "$1/gpy-agent"
}

write_stub "$TMP_DIR/home/.local/bin" home
write_stub "$TMP_DIR/path" path

# run PATH: resolve, oneshot and start in a fresh shell with HOME sandboxed.
run() {
    # shellcheck disable=SC2016 # the script expands in the child shell
    env -i HOME="$TMP_DIR/home" PATH="$1" LOG="$LOG" GPY_AGENT_ENABLED=1 \
        GPY_AGENT_SOCKET_PATH="$TMP_DIR/none.sock" bash --norc --noprofile -c '
        source bash/core/constants.bash
        source bash/core/renderer.bash
        source bash/core/ipc.bash
        source bash/core/supervisor.bash
        __gpy_agent_binary_into agent; echo "resolve=$? $agent"
        echo "oneshot=$(__gpy_fallback_oneshot directory / bash-prompt)"
        __gpy_supervisor_start; echo "start=$?"
        wait
    ' 2>/dev/null
}

: >"$LOG"
out="$(run /usr/bin:/bin)"
check "without gpy-agent on PATH, ~/.local/bin is used" \
    "resolve=0 $TMP_DIR/home/.local/bin/gpy-agent" "$(sed -n 1p <<<"$out")"
check "oneshot runs the ~/.local/bin copy" "oneshot=home" "$(sed -n 2p <<<"$out")"
check "start runs the ~/.local/bin copy" "start=0" "$(sed -n 3p <<<"$out")"
# The start is detached: allow it a moment to log.
for _ in 1 2 3 4 5 6 7 8 9 10; do
    grep -q '^home start' "$LOG" && break
    sleep 0.1
done
check "the start reached the ~/.local/bin copy" 1 "$(grep -c '^home start' "$LOG")"

: >"$LOG"
out="$(run "$TMP_DIR/path:/usr/bin:/bin")"
check "a gpy-agent on PATH wins" "resolve=0 gpy-agent" "$(sed -n 1p <<<"$out")"
check "oneshot runs the PATH copy" "oneshot=path" "$(sed -n 2p <<<"$out")"

rm -f "$TMP_DIR/home/.local/bin/gpy-agent"
out="$(run /usr/bin:/bin)"
check "with neither, resolution fails" "resolve=1 " "$(sed -n 1p <<<"$out")"
check "with neither, start reports failure" "start=1" "$(sed -n 3p <<<"$out")"

test_finish "gpy-agent falls back to ~/.local/bin"
