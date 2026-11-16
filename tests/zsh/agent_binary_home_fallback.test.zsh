#!/usr/bin/env zsh
# tests/zsh/agent_binary_home_fallback.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The installer puts gpy-agent in ~/.local/bin, which is not always on PATH.
# As in Fish, the shell then runs that copy for start, theme export and
# oneshot; a gpy-agent on PATH still wins.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-home-fallback.XXXXXX") || exit 1
tmp=${tmp:A}
trap 'rm -rf "$tmp"' EXIT
log="$tmp/calls"

# write_stub DIR TAG: a gpy-agent that logs "TAG <argv>" and prints TAG on oneshot.
write_stub() {
    mkdir -p "$1"
    print -r -- "#!/bin/sh
echo \"$2 \$*\" >>'$log'
[ \"\$1\" = oneshot ] && printf '%s' '$2'
exit 0" >"$1/gpy-agent"
    chmod +x "$1/gpy-agent"
}

write_stub "$tmp/home/.local/bin" home
write_stub "$tmp/path" path

# run PATH: resolve, oneshot and start in a fresh shell with HOME sandboxed.
run() {
    env -i HOME="$tmp/home" PATH="$1" GPY_AGENT_ENABLED=1 \
        GPY_AGENT_SOCKET_PATH="$tmp/none.sock" zsh -f -c '
        source zsh/core/constants.zsh
        source zsh/core/renderer.zsh
        source zsh/core/ipc.zsh
        source zsh/core/supervisor.zsh
        __gpy_agent_binary_into agent; print -r -- "resolve=$? $agent"
        print -r -- "oneshot=$(__gpy_fallback_oneshot directory / zsh-prompt)"
        __gpy_start_agent; print -r -- "start=$?"
    ' 2>/dev/null
}

: >"$log"
out=("${(@f)$(run /usr/bin:/bin)}")
check "without gpy-agent on PATH, ~/.local/bin is used" \
    "resolve=0 $tmp/home/.local/bin/gpy-agent" "$out[1]"
check "oneshot runs the ~/.local/bin copy" "oneshot=home" "$out[2]"
check "start runs the ~/.local/bin copy" "start=0" "$out[3]"
# The start is detached: allow it a moment to log.
for _ in {1..10}; do
    grep -q '^home start' "$log" && break
    sleep 0.1
done
check "the start reached the ~/.local/bin copy" 1 "$(grep -c '^home start' "$log")"

: >"$log"
out=("${(@f)$(run "$tmp/path:/usr/bin:/bin")}")
check "a gpy-agent on PATH wins" "resolve=0 gpy-agent" "$out[1]"
check "oneshot runs the PATH copy" "oneshot=path" "$out[2]"

rm -f "$tmp/home/.local/bin/gpy-agent"
out=("${(@f)$(run /usr/bin:/bin)}")
check "with neither, resolution fails" "resolve=1 " "$out[1]"
check "with neither, start reports failure" "start=1" "$out[3]"

test_finish "gpy-agent falls back to ~/.local/bin"
