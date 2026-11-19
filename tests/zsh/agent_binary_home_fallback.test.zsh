#!/usr/bin/env zsh
# tests/zsh/agent_binary_home_fallback.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The installer puts gpy in ~/.local/bin, which is not always on PATH. The
# prompt then runs that copy, for the prompt and for the unregister on exit; a
# gpy on PATH still wins. With neither, the prompt is the bare `❯ `.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-home-fallback.XXXXXX") || exit 1
tmp=${tmp:A}
trap 'rm -rf "$tmp"' EXIT
log="$tmp/calls"

# write_stub DIR TAG: a gpy that logs "TAG <argv>" and prints a prompt
# "TAG> " with the transient line "t".
write_stub() {
    mkdir -p "$1"
    print -r -- "#!/bin/sh
echo \"$2 \$*\" >>'$log'
[ \"\$2\" = --exit ] && exit 0
printf '%s\\037%s' '$2> ' t" >"$1/gpy"
    chmod +x "$1/gpy"
}

write_stub "$tmp/home/.local/bin" home
write_stub "$tmp/path" path

# run PATH: one prompt and an exit in a fresh shell with HOME sandboxed.
run() {
    env -i HOME="$tmp/home" PATH="$1" zsh -f -c '
        source zsh/gpy.zsh
        __gpy_precmd
        print -r -- "prompt=$PROMPT"
    ' 2>/dev/null
}

: >"$log"
check "without gpy on PATH, the ~/.local/bin copy draws the prompt" \
    "prompt=home> " "$(run /usr/bin:/bin)"
check "exit unregisters through the ~/.local/bin copy" 1 "$(grep -c '^home prompt zsh --exit' "$log")"

: >"$log"
check "a gpy on PATH wins" "prompt=path> " "$(run "$tmp/path:/usr/bin:/bin")"
check "the ~/.local/bin copy is not run" 0 "$(grep -c '^home' "$log")"

rm -f "$tmp/home/.local/bin/gpy"
check "with neither, the prompt is the bare symbol" "prompt=❯ " "$(run /usr/bin:/bin)"

test_finish "gpy falls back to ~/.local/bin"
