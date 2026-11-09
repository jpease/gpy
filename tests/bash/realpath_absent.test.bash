#!/usr/bin/env bash
# tests/bash/realpath_absent.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# The Bash path helpers work without realpath(1) (absent before macOS 13):
# git root and language cache keys still resolve symlinks like the agent.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

S="$(mktemp -d "${TMPDIR:-/tmp}/gpy-rp-absent.XXXXXX")" || exit 1
trap 'rm -rf "$S"' EXIT
mkdir -p "$S/repo/.git" "$S/repo/sub" "$S/plain/sub" "$S/bin"
ln -s repo "$S/link"
ln -s plain "$S/link_plain"

# A realpath that fails and records any call, first on PATH.
printf '#!/bin/sh\necho called >> "%s/rp.log"\nexit 127\n' "$S" >"$S/bin/realpath"
chmod +x "$S/bin/realpath"
PATH="$S/bin:$PATH"

real() { python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"; }

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

want="$(real "$S/repo")"
cd "$S/link/sub" || exit 1
__gpy_resolve_pwd
r=""
__gpy_find_git_root "$PWD" r
rc=$?
if [[ $rc -eq 0 && "$r" == "$want" ]]; then
    pass "symlinked \$PWD resolves to the real git root without realpath"
else
    fail "symlinked \$PWD: rc=$rc root=[$r] want=[$want]"
fi

r=""
__gpy_find_git_root "$S/link/sub" r
if [[ "$r" == "$want" ]]; then
    pass "a non-PWD start resolves without realpath"
else
    fail "non-PWD start: root=[$r] want=[$want]"
fi

want="$(real "$S/plain/sub")"
cd "$S/link_plain/sub" || exit 1
__gpy_resolve_pwd
k=""
__gpy_instant_cache_key_path lang "$PWD" k
if [[ "$k" == "$want" ]]; then
    pass "the language key outside git is the canonical cwd"
else
    fail "language key: [$k] want=[$want]"
fi

r=""
__gpy_find_git_root "$S/nope" r
rc=$?
if [[ $rc -eq 1 && -z "$r" ]]; then
    pass "a missing directory finds no root"
else
    fail "missing directory: rc=$rc root=[$r]"
fi

if [[ ! -e "$S/rp.log" ]]; then
    pass "realpath is never invoked"
else
    fail "realpath was invoked"
fi

# A user's `cd` wrapper (zoxide, nvm, ...) must stay out of the resolution.
# shellcheck disable=SC2329 # shadows the builtin on purpose
cd() {
    echo called >>"$S/cd.log"
    return 1
}
r=""
__gpy_find_git_root "$S/link/sub" r
rc=$?
if [[ $rc -eq 0 && "$r" == "$(real "$S/repo")" && ! -e "$S/cd.log" ]]; then
    pass "a user-defined cd is never invoked and resolution still works"
else
    fail "cd wrapper: rc=$rc root=[$r] cd.log=$([[ -e "$S/cd.log" ]] && echo present || echo absent)"
fi
unset -f cd

test_finish
