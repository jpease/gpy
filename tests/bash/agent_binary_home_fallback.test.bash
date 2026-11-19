#!/usr/bin/env bash
# tests/bash/agent_binary_home_fallback.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The installer puts gpy in ~/.local/bin, which is not always on PATH. The
# prompt hook then runs that copy; a gpy on PATH still wins; with neither,
# the prompt is the bare `❯ `.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
LOG="$TMP_DIR/calls"

# write_stub DIR TAG: a gpy that logs "TAG <argv>" and prints "TAG> " as the prompt.
write_stub() {
    mkdir -p "$1"
    cat >"$1/gpy" <<STUB
#!/bin/sh
echo "$2 \$*" >>"$LOG"
[ "\$1" = prompt ] && [ "\$3" != --exit ] && printf '%s> ' "$2"
exit 0
STUB
    chmod +x "$1/gpy"
}

write_stub "$TMP_DIR/home/.local/bin" home
write_stub "$TMP_DIR/path" path

# run PATH: one prompt in a fresh shell with HOME sandboxed; prints PS1.
run() {
    # shellcheck disable=SC2016 # the script expands in the child shell
    env -i HOME="$TMP_DIR/home" PATH="$1" LOG="$LOG" "$BASH" --norc --noprofile -c '
        source bash/gpy.bash
        __gpy_precmd
        printf "PS1=%s\n" "$PS1"
    ' 2>/dev/null
}

: >"$LOG"
check "without gpy on PATH, ~/.local/bin is used" "PS1=home> " "$(run /usr/bin:/bin)"
check "the prompt ran the ~/.local/bin copy" 1 "$(grep -c '^home prompt bash --status' "$LOG")"
check "exit unregistered through the ~/.local/bin copy" 1 "$(grep -c '^home prompt bash --exit --pid' "$LOG")"

: >"$LOG"
check "a gpy on PATH wins" "PS1=path> " "$(run "$TMP_DIR/path:/usr/bin:/bin")"
check "the ~/.local/bin copy is not run when PATH has gpy" 0 "$(grep -c '^home' "$LOG")"

rm -f "$TMP_DIR/home/.local/bin/gpy"
check "with neither, the prompt is the bare symbol" "PS1=❯ " "$(run /usr/bin:/bin)"

test_finish "gpy falls back to ~/.local/bin"
