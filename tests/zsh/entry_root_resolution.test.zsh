#!/usr/bin/env zsh
# tests/zsh/entry_root_resolution.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# zsh/gpy.zsh finds its own tree however it is sourced: by a bare relative
# name from its own directory (`${0%/*}` gave `gpy.zsh` there and disabled
# GPY), through a symlink to the entry file alone, and from inside a function
# with NO_FUNCTION_ARGZERO (where $0 is not the file). The completions dir it
# adds to fpath is absolute.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-entry-root.XXXXXX") || exit 1
SANDBOX=${SANDBOX:A}
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/home" "$SANDBOX/cache" "$SANDBOX/run" "$SANDBOX/cfg" "$SANDBOX/link"
ln -s "$ROOT/zsh/gpy.zsh" "$SANDBOX/link/gpy.zsh"

# load_case DIR BODY: BODY sources gpy.zsh from DIR in a clean, agent-free zsh;
# prints `loaded=<0|1> fpath0=<first fpath entry>` and any stderr.
load_case() {
    (cd "$1" && env -i HOME="$SANDBOX/home" PATH=/usr/bin:/bin TMPDIR="$SANDBOX" \
        XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" XDG_CONFIG_HOME="$SANDBOX/cfg" \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        SANDBOX="$SANDBOX" zsh -f -c "$2"' 2>&1
            print -r -- "loaded=${+functions[__gpy_precmd]} fpath0=${fpath[1]}"')
}

out=$(load_case "$ROOT/zsh" 'source gpy.zsh')
check "a relative source from its own directory loads" true \
    "$([[ $out == *loaded=1* && $out != *"core files not found"* ]] && echo true || echo false)"
check "and puts an absolute completions dir on fpath" true \
    "$([[ $out == *"fpath0=$ROOT/zsh/completions"* ]] && echo true || echo false)"

out=$(load_case "$SANDBOX" 'source link/gpy.zsh')
check "a symlink to the entry file alone loads the real tree" true \
    "$([[ $out == *loaded=1* ]] && echo true || echo false)"

out=$(load_case "$SANDBOX" 'setopt no_function_argzero; load() { source "$SANDBOX/link/gpy.zsh"; }; load')
check "sourcing from a function under NO_FUNCTION_ARGZERO loads" true \
    "$([[ $out == *loaded=1* ]] && echo true || echo false)"

test_finish "gpy.zsh resolves its own tree"
