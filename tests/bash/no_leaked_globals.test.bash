#!/usr/bin/env bash
# tests/bash/no_leaked_globals.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Loading bash/gpy.bash leaves no entry-point variable behind (the checkout
# root, the segment loop variable) and adds no variable outside the private
# `__` namespace except a short, spelled-out allowlist. The shell starts under
# `env -i`: the test harnesses export GPY_ROOT, which would hide a leak.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-leaked-globals.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home" "$tmp/cache" "$tmp/run"

# shellcheck disable=SC2016  # the program is single-quoted; $ROOT and $TMPDIR expand in the child
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TMPDIR="$tmp" ROOT="$ROOT" \
    XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run" GPY_AGENT_SUPERVISOR_ENABLED=0 \
    "$BASH" --noprofile --norc -c '
        compgen -v | sort >"$TMPDIR/before"
        source "$ROOT/bash/gpy.bash" >/dev/null 2>&1
        compgen -v | sort >"$TMPDIR/after"
    '

# The load happened: a name the integration always adds is there.
if grep -qx __gpy_fallback_segments "$tmp/after"; then
    pass "gpy.bash loaded in the env -i shell"
else
    fail "gpy.bash did not load in the env -i shell"
fi

for name in segment core GPY_BASH_ROOT GPY_ROOT __gpy_bash_root __gpy_zsh_root __gpy_seg_file; do
    if grep -qx "$name" "$tmp/after"; then
        fail "$name is still set after loading"
    else
        pass "$name is unset after loading"
    fi
done

# Shell-managed names, plus the documented GPY_ tunables constants.bash seeds
# and the oneshot status channel; every other added name starts with __.
allowed=" BASH_REMATCH PIPESTATUS PROMPT_COMMAND GPY_EXPECTED_PROTOCOL_VERSION GPY_GIT_INSTANT_CACHE_TTL_SECONDS GPY_IPC_COLD_TIMEOUT_MS GPY_IPC_TIMEOUT_MS GPY_LANGUAGE_CACHE_TTL_SECONDS GPY_SEG_STATUS_ONESHOT "
unexpected=""
for name in $(comm -13 "$tmp/before" "$tmp/after"); do
    case "$name" in __*) continue ;; esac
    case "$allowed" in *" $name "*) continue ;; esac
    unexpected="$unexpected $name"
done
check "every added variable is private (__) or allowlisted" "" "${unexpected# }"

test_finish "gpy.bash leaks no global variables"
