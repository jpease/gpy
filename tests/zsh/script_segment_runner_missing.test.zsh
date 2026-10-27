#!/usr/bin/env zsh
# tests/zsh/script_segment_runner_missing.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# When g_runner.bash is missing, zsh/core/script.zsh turns script segments off
# and emits a warning on stderr if GPY_DEBUG=1 or GPY_VERBOSE=1, matching Fish.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-runner-missing.XXXXXX") || exit 1
WORKDIR=${WORKDIR:A}
trap 'rm -rf "$WORKDIR"' EXIT

mkdir -p "$WORKDIR/core"
cp "$ROOT/zsh/core/script.zsh" "$WORKDIR/core/script.zsh"
# g_runner.bash is intentionally omitted under $WORKDIR/core

run_with() {
    (
        eval "$1"
        source "$WORKDIR/core/script.zsh"
        print -r -- "src=$__gpy_g_src"
    ) 2>&1
}

runner_file="$WORKDIR/core/g_runner.bash"
expected_warn="gpy[script]: warning: Runner $runner_file missing - script segments disabled"

check "missing runner, GPY_DEBUG=1: one warning, runner disabled" \
    "$expected_warn|src=" \
    "$(run_with 'export GPY_DEBUG=1' | paste -sd'|' -)"

check "missing runner, GPY_VERBOSE=1: the same warning" \
    "$expected_warn|src=" \
    "$(run_with 'export GPY_VERBOSE=1' | paste -sd'|' -)"

check "missing runner, no debug flags: silent" \
    "src=" \
    "$(run_with 'unset GPY_DEBUG GPY_VERBOSE')"

test_finish "script segment runner missing"
