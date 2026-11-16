#!/usr/bin/env zsh
# tests/zsh/precmd_hostile_options.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The prompt hooks run with zsh's own options (`emulate -L zsh`), so a user's
# KSH_ARRAYS (1-based indexing and ${#array} break otherwise: every segment but
# the symbol vanished, and a stale instant-cache entry read as fresh), SH_GLOB
# or ERR_RETURN cannot break the render, and the user's options are unchanged
# afterwards. prompt_bang, which PROMPT expansion reads after precmd returns,
# keeps the user's setting inside the hooks.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-hostile-opts.XXXXXX") || exit 1
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/home" "$SANDBOX/cache" "$SANDBOX/run" "$SANDBOX/cfg"

out=$(env -i HOME="$SANDBOX/home" PATH=/usr/bin:/bin TMPDIR="$SANDBOX" ROOT="$ROOT" \
    XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" XDG_CONFIG_HOME="$SANDBOX/cfg" \
    GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
    zsh -f -c '
        source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1
        __gpy_add_newline=0 __gpy_two_line=0
        __icon_status_ok=OKICON
        __enabled_segments=(status clock)
        setopt ksh_arrays sh_glob err_return prompt_bang
        true
        __gpy_precmd
        print -r -- "status=$([[ $PROMPT == *OKICON* ]] && echo 1 || echo 0)"
        print -r -- "clock=$([[ $PROMPT == *"%D{"* ]] && echo 1 || echo 0)"
        print -r -- "ksh=$([[ -o ksh_arrays ]] && echo 1 || echo 0)"
        print -r -- "shglob=$([[ -o sh_glob ]] && echo 1 || echo 0)"
        print -r -- "bang=$([[ -o prompt_bang ]] && echo 1 || echo 0)"
    ' 2>&1)

field() { print -r -- "$out" | sed -n "s/^$1=//p"; }
check "under KSH_ARRAYS the status pill renders" 1 "$(field status)"
check "under KSH_ARRAYS the clock renders" 1 "$(field clock)"
check "the user's KSH_ARRAYS is still set afterwards" 1 "$(field ksh)"
check "the user's SH_GLOB is still set afterwards" 1 "$(field shglob)"
check "the user's PROMPT_BANG is still set afterwards" 1 "$(field bang)"

test_finish "precmd renders under hostile user options"
