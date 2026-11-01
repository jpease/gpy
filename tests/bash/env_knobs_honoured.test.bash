#!/usr/bin/env bash
# tests/bash/env_knobs_honoured.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY_IPC_TIMEOUT_MS, GPY_IPC_COLD_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment, read at each use site (twins in tests/fish, tests/zsh).
# constants.bash assigns none of them: a shell never writes the user's variables.
#

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# constants_with VAR=VALUE...: the four knobs as a child bash sees them after
# sourcing constants.bash; `-` is unset.
# shellcheck disable=SC2016 # the child shell expands its own variables
constants_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_IPC_COLD_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS "$@" \
        bash -c '. "$1/bash/core/constants.bash"; echo "${GPY_IPC_TIMEOUT_MS-} ${GPY_IPC_COLD_TIMEOUT_MS-} ${GPY_GIT_INSTANT_CACHE_TTL_SECONDS-} ${GPY_LANGUAGE_CACHE_TTL_SECONDS-}"' _ "$ROOT"
}

check "constants.bash assigns no knob when nothing is set" "   " "$(constants_with)"
check "constants.bash leaves a set knob alone" "750   " "$(constants_with GPY_IPC_TIMEOUT_MS=750)"

# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash
secs=""
__gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-}" secs
check "an unset IPC budget is the 150 ms default" "0.150" "$secs"
__gpy_ms_to_secs 750 secs
check "an IPC budget of 750 ms is 0.750 s" "0.750" "$secs"
__gpy_ms_to_secs fast secs
check "a non-numeric IPC budget falls back to 150 ms" "0.150" "$secs"

# The cold budget (nothing to show yet): 500 ms by default, honoured when set,
# never below GPY_IPC_TIMEOUT_MS, and the default when non-numeric.
# shellcheck disable=SC2016 # the child shell expands its own variables
cold_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_IPC_COLD_TIMEOUT_MS "$@" bash -c \
        '. "$1/bash/core/constants.bash"; . "$1/bash/core/ipc.bash"; __gpy_cold_timeout_ms_into ms; echo "$ms"' _ "$ROOT"
}
check "the cold budget defaults to 500 ms" "500" "$(cold_with)"
check "GPY_IPC_COLD_TIMEOUT_MS is honoured" "900" "$(cold_with GPY_IPC_COLD_TIMEOUT_MS=900)"
check "the cold budget is never below GPY_IPC_TIMEOUT_MS" "2000" "$(cold_with GPY_IPC_TIMEOUT_MS=2000)"
check "a non-numeric cold budget falls back to 500 ms" "500" "$(cold_with GPY_IPC_COLD_TIMEOUT_MS=slow)"

# The TTL knobs: an entry 8 s old is stale under the git default (5 s) and
# fresh under the language default (30 s); each knob moves its own line.
tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-knobs.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
export XDG_CACHE_HOME="$tmp/cache"
mkdir -p "$tmp/repo/.git" "$XDG_CACHE_HOME/gpy/instant-prompts"
key=""
__gpy_path_to_cache_key "$(realpath "$tmp/repo")" key
entry_git="$XDG_CACHE_HOME/gpy/instant-prompts/$key.git.none.bash"
entry_lang="$XDG_CACHE_HOME/gpy/instant-prompts/$key.lang.none.bash"
printf 'cached' >"$entry_git"
printf 'cached' >"$entry_lang"
age_entries() {
    python3 -c 'import os, sys, time
t = time.time() - int(sys.argv[1])
for p in sys.argv[2:]:
    os.utime(p, (t, t))' "$1" "$entry_git" "$entry_lang"
}
read_status() {
    __gpy_read_instant_cache "$1" "$tmp/repo" "" ignored 2>/dev/null
    echo $?
}
age_entries 8
unset GPY_GIT_INSTANT_CACHE_TTL_SECONDS GPY_LANGUAGE_CACHE_TTL_SECONDS
check "git: the default TTL (5 s) ages an 8 s entry to stale" "2" "$(read_status git)"
check "lang: the default TTL (30 s) keeps an 8 s entry fresh" "0" "$(read_status lang)"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" "0" "$(read_status git)"
export GPY_LANGUAGE_CACHE_TTL_SECONDS=5
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" "2" "$(read_status lang)"

# Non-numeric TTL: default 5 s applies, so a 60 s-old entry is stale.
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=soon
age_entries 1
check "a non-numeric TTL still reads a fresh entry as fresh" "0" "$(read_status git)"
age_entries 60
check "a non-numeric TTL falls back to the default and ages an old entry to stale" "2" "$(read_status git)"

test_finish "bash env knobs"
