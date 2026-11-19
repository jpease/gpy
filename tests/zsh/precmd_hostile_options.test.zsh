#!/usr/bin/env zsh
# tests/zsh/precmd_hostile_options.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The prompt hooks run with zsh's own options (`emulate -L zsh`), so a user's
# KSH_ARRAYS (1-based indexing and ${#array} break otherwise), SH_GLOB or
# ERR_RETURN cannot break the render: the binary still gets the last status,
# and PROMPT and the transient line are split from its output. The user's
# options are unchanged afterwards. prompt_bang, which PROMPT expansion reads
# after precmd returns, keeps the user's setting inside the hooks.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-hostile-opts.XXXXXX") || exit 1
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/home" "$SANDBOX/bin"

# A gpy stub: `prompt zsh --status N ...` prints "S<N>> ", 0x1F, "T".
print -r -- '#!/bin/sh
[ "$3" = --status ] || exit 1
printf "S%s> \037T" "$4"' >"$SANDBOX/bin/gpy"
chmod +x "$SANDBOX/bin/gpy"

out=$(env -i HOME="$SANDBOX/home" PATH="$SANDBOX/bin:/usr/bin:/bin" ROOT="$ROOT" \
    zsh -f -c '
        source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1
        setopt ksh_arrays sh_glob err_return prompt_bang
        f() { return 3 }
        # Top-level ERR_RETURN exits the shell on a failure outside `||`.
        f || __gpy_precmd
        print -r -- "prompt=$PROMPT"
        print -r -- "transient=$__gpy_transient"
        print -r -- "ksh=$([[ -o ksh_arrays ]] && echo 1 || echo 0)"
        print -r -- "shglob=$([[ -o sh_glob ]] && echo 1 || echo 0)"
        print -r -- "errreturn=$([[ -o err_return ]] && echo 1 || echo 0)"
        print -r -- "bang=$([[ -o prompt_bang ]] && echo 1 || echo 0)"
    ' 2>&1)

field() { print -r -- "$out" | sed -n "s/^$1=//p"; }
check "under hostile options the prompt carries the last status" "S3> " "$(field prompt)"
check "under hostile options the transient line is split off" "T" "$(field transient)"
check "the user's KSH_ARRAYS is still set afterwards" 1 "$(field ksh)"
check "the user's SH_GLOB is still set afterwards" 1 "$(field shglob)"
check "the user's ERR_RETURN is still set afterwards" 1 "$(field errreturn)"
check "the user's PROMPT_BANG is still set afterwards" 1 "$(field bang)"

test_finish "precmd renders under hostile user options"
