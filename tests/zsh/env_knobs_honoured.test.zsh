#!/usr/bin/env zsh
# tests/zsh/env_knobs_honoured.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_IPC_TIMEOUT_MS, GPY_IPC_COLD_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment before the shell integration loads, identically in all
# three shells (tests/fish/env_knobs_honoured.test.fish and
# tests/bash/env_knobs_honoured.test.bash are the twins). Sourcing
# zsh/core/constants.zsh must not overwrite an exported GPY_IPC_TIMEOUT_MS.
# A non-numeric IPC budget falls back to 150 ms inside __gpy_ms_to_secs.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

# constants_with VAR=VALUE...: "ipc git lang" as a child zsh sees them.
constants_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS "$@" \
        zsh -f -c 'source "$1/zsh/core/constants.zsh"; print -r -- "$GPY_IPC_TIMEOUT_MS $GPY_GIT_INSTANT_CACHE_TTL_SECONDS $GPY_LANGUAGE_CACHE_TTL_SECONDS"' _ "$ROOT"
}

check "defaults when nothing is set" "150 5 30" "$(constants_with)"
check "GPY_IPC_TIMEOUT_MS is honoured" "750 5 30" "$(constants_with GPY_IPC_TIMEOUT_MS=750)"
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" "150 11 30" "$(constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11)"
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" "150 5 22" "$(constants_with GPY_LANGUAGE_CACHE_TTL_SECONDS=22)"

source zsh/core/ipc.zsh
__gpy_ms_to_secs 750 secs
check "an IPC budget of 750 ms is 0.750 s" "0.750" "$secs"
__gpy_ms_to_secs fast secs
check "a non-numeric IPC budget falls back to 150 ms" "0.150" "$secs"

# The cold budget (nothing to show yet): 500 ms by default, honoured when set,
# never below GPY_IPC_TIMEOUT_MS, and the default when non-numeric.
cold_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_IPC_COLD_TIMEOUT_MS "$@" zsh -f -c \
        'source "$1/zsh/core/constants.zsh"; source "$1/zsh/core/ipc.zsh"; __gpy_cold_timeout_ms_into ms; print -r -- "$ms"' _ "$ROOT"
}
check "the cold budget defaults to 500 ms" "500" "$(cold_with)"
check "GPY_IPC_COLD_TIMEOUT_MS is honoured" "900" "$(cold_with GPY_IPC_COLD_TIMEOUT_MS=900)"
check "the cold budget is never below GPY_IPC_TIMEOUT_MS" "2000" "$(cold_with GPY_IPC_TIMEOUT_MS=2000)"
check "a non-numeric cold budget falls back to 500 ms" "500" "$(cold_with GPY_IPC_COLD_TIMEOUT_MS=slow)"

# A non-numeric TTL must not break the freshness comparison: the entry is read
# with the default TTL (git: 5 s), so an old entry is stale and a fresh one is not.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-knobs.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
export XDG_CACHE_HOME="$tmp/cache"
mkdir -p "$tmp/repo/.git" "$XDG_CACHE_HOME/gpy/instant-prompts"
__gpy_path_to_cache_key "$(realpath "$tmp/repo")" key
entry="$XDG_CACHE_HOME/gpy/instant-prompts/$key.git.none.zsh"
print -rn -- cached >"$entry"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=soon
__gpy_read_instant_cache git "$tmp/repo" "" ignored 2>/dev/null
check "a non-numeric TTL still reads a fresh entry as fresh" "0" "$?"
touch -t 200001010000 "$entry"
__gpy_read_instant_cache git "$tmp/repo" "" ignored 2>/dev/null
check "a non-numeric TTL falls back to the default and ages an old entry to stale" "2" "$?"

test_finish "zsh env knobs"
