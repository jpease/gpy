#!/usr/bin/env zsh
# tests/zsh/env_knobs_honoured.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_IPC_TIMEOUT_MS, GPY_IPC_COLD_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment, read at each use site, identically in all three shells
# (tests/fish/env_knobs_honoured.test.fish and
# tests/bash/env_knobs_honoured.test.bash are the twins). zsh/core/constants.zsh
# assigns none of them: a shell never writes the user's variables.
# A non-numeric IPC budget falls back to 150 ms inside __gpy_ms_to_secs.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

# constants_with VAR=VALUE...: the four knobs as a child zsh sees them after
# sourcing constants.zsh.
constants_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_IPC_COLD_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS "$@" \
        zsh -f -c 'source "$1/zsh/core/constants.zsh"; print -r -- "${GPY_IPC_TIMEOUT_MS-} ${GPY_IPC_COLD_TIMEOUT_MS-} ${GPY_GIT_INSTANT_CACHE_TTL_SECONDS-} ${GPY_LANGUAGE_CACHE_TTL_SECONDS-}"' _ "$ROOT"
}

check "constants.zsh assigns no knob when nothing is set" "   " "$(constants_with)"
check "constants.zsh leaves a set knob alone" "750   " "$(constants_with GPY_IPC_TIMEOUT_MS=750)"

source zsh/core/ipc.zsh
__gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-}" secs
check "an unset IPC budget is the 150 ms default" "0.150" "$secs"
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

# The TTL knobs: an entry 8 s old is stale under the git default (5 s) and
# fresh under the language default (30 s); each knob moves its own line. A
# non-numeric TTL must not break the freshness comparison.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-knobs.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
export XDG_CACHE_HOME="$tmp/cache"
mkdir -p "$tmp/repo/.git" "$XDG_CACHE_HOME/gpy/instant-prompts"
__gpy_path_to_cache_key "$(realpath "$tmp/repo")" key
entry_git="$XDG_CACHE_HOME/gpy/instant-prompts/$key.git.none.zsh"
entry_lang="$XDG_CACHE_HOME/gpy/instant-prompts/$key.lang.none.zsh"
print -rn -- cached >"$entry_git"
print -rn -- cached >"$entry_lang"
age_entries() {
    python3 -c 'import os, sys, time
t = time.time() - int(sys.argv[1])
for p in sys.argv[2:]:
    os.utime(p, (t, t))' "$1" "$entry_git" "$entry_lang"
}
read_status() {
    __gpy_read_instant_cache "$1" "$tmp/repo" "" ignored 2>/dev/null
    print -r -- $?
}
age_entries 8
unset GPY_GIT_INSTANT_CACHE_TTL_SECONDS GPY_LANGUAGE_CACHE_TTL_SECONDS
check "git: the default TTL (5 s) ages an 8 s entry to stale" "2" "$(read_status git)"
check "lang: the default TTL (30 s) keeps an 8 s entry fresh" "0" "$(read_status lang)"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" "0" "$(read_status git)"
export GPY_LANGUAGE_CACHE_TTL_SECONDS=5
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" "2" "$(read_status lang)"

export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=soon
age_entries 1
check "a non-numeric TTL still reads a fresh entry as fresh" "0" "$(read_status git)"
age_entries 60
check "a non-numeric TTL falls back to the default and ages an old entry to stale" "2" "$(read_status git)"

test_finish "zsh env knobs"
