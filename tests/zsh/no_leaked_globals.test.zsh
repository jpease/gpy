#!/usr/bin/env zsh
# tests/zsh/no_leaked_globals.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Loading zsh/gpy.zsh leaves no entry-point variable behind (the checkout
# root, the segment loop variable) and adds no parameter outside the private
# `__` namespace except a short, spelled-out allowlist. The shell starts under
# `env -i`: the test harnesses export GPY_ROOT, which would hide a leak.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-leaked-globals.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home" "$tmp/cache" "$tmp/run"

# Parameter names: the last field of each `typeset +m '*'` line, because
# non-scalars print as `array name` or `association name`.
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TMPDIR="$tmp" ROOT="$ROOT" \
    XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run" GPY_AGENT_SUPERVISOR_ENABLED=0 \
    zsh -f -c '
        names() { typeset +m "*" | awk "{print \$NF}" | sort; }
        names >"$TMPDIR/before"
        source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1
        names >"$TMPDIR/after"
    '

# The load happened: a name the integration always adds is there.
if grep -qx __gpy_fallback_segments "$tmp/after"; then
    pass "gpy.zsh loaded in the env -i shell"
else
    fail "gpy.zsh did not load in the env -i shell"
fi

for name in segment core GPY_BASH_ROOT GPY_ROOT __gpy_bash_root __gpy_zsh_root __gpy_seg_file; do
    if grep -qx "$name" "$tmp/after"; then
        fail "$name is still set after loading"
    else
        pass "$name is unset after loading"
    fi
done

# Shell-managed names, plus the documented GPY_ tunables constants.zsh seeds
# and the oneshot status channel; every other added name starts with __.
allowed=(EPOCHREALTIME EPOCHSECONDS epochtime errnos precmd_functions preexec_functions sysparams zle_bracketed_paste zshexit_functions
    GPY_EXPECTED_PROTOCOL_VERSION GPY_GIT_INSTANT_CACHE_TTL_SECONDS GPY_IPC_COLD_TIMEOUT_MS GPY_IPC_TIMEOUT_MS GPY_LANGUAGE_CACHE_TTL_SECONDS GPY_SEG_STATUS_ONESHOT)
unexpected=()
for name in ${(f)"$(comm -13 "$tmp/before" "$tmp/after")"}; do
    [[ $name == __* ]] && continue
    (( ${allowed[(Ie)$name]} )) && continue
    unexpected+=($name)
done
check "every added parameter is private (__) or allowlisted" "" "${unexpected[*]}"

test_finish "gpy.zsh leaks no global variables"
