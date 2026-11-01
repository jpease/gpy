#!/usr/bin/env zsh
# tests/zsh/script_runner_symlinked_tree.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# When the Zsh tree is a symlink to a directory that has no sibling bash/,
# zsh/core/script.zsh still finds bash/core/g_runner.bash beside the link:
# the fallback path is built lexically (`:a:h:h`), not through the kernel's
# physical `..`.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-runner-symlink.XXXXXX") || exit 1
WORKDIR=${WORKDIR:A}
trap 'rm -rf "$WORKDIR"' EXIT

mkdir -p "$WORKDIR/data/bash/core" "$WORKDIR/dot/zsh-real/core"
cp "$ROOT/zsh/core/script.zsh" "$WORKDIR/dot/zsh-real/core/script.zsh"
cp "$ROOT/bash/core/g_runner.bash" "$WORKDIR/data/bash/core/g_runner.bash"
ln -s "$WORKDIR/dot/zsh-real" "$WORKDIR/data/zsh"

got=$(
    source "$WORKDIR/data/zsh/core/script.zsh" 2>/dev/null
    print -r -- "$__gpy_g_runner_file"
)
check "a symlinked zsh tree finds the sibling bash/core runner" \
    "$WORKDIR/data/bash/core/g_runner.bash" "$got"
check "no core directory variable is left behind" "" \
    "$(source "$WORKDIR/data/zsh/core/script.zsh" 2>/dev/null; print -r -- ${(t)__gpy_g_core_dir})"

test_finish "script runner lookup through a symlinked tree"
