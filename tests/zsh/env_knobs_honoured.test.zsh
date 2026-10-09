#!/usr/bin/env zsh
# tests/zsh/env_knobs_honoured.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_IPC_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment before the shell integration loads, identically in all
# three shells (#845; tests/fish/env_knobs_honoured.test.fish and
# tests/bash/env_knobs_honoured.test.bash are the twins).
# zsh/core/constants.zsh used to overwrite GPY_IPC_TIMEOUT_MS with 150 at
# source time, so exporting it in .zshrc did nothing (the TTLs were honoured).
# A non-numeric IPC budget falls back to 150 ms inside __gpy_ms_to_secs.

ROOT=${0:a:h:h:h}
cd "$ROOT"

failures=0
check() {
    if [[ "$3" == "$2" ]]; then
        echo "PASS: $1"
    else
        echo "FAIL: $1 (expected [$2], got [$3])"
        failures=$((failures + 1))
    fi
}

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

# A non-numeric TTL must not break the freshness comparison: the entry is read
# with the default TTL (git: 5 s), so an old entry is stale and a fresh one is not.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-knobs.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
export XDG_CACHE_HOME="$tmp/cache"
mkdir -p "$tmp/repo/.git" "$XDG_CACHE_HOME/gpy/instant-prompts"
key=$(__gpy_path_to_cache_key "$(realpath "$tmp/repo")")
entry="$XDG_CACHE_HOME/gpy/instant-prompts/$key.git.none.zsh"
print -rn -- cached >"$entry"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=soon
__gpy_read_instant_cache git "$tmp/repo" >/dev/null 2>&1
check "a non-numeric TTL still reads a fresh entry as fresh" "0" "$?"
touch -t 200001010000 "$entry"
__gpy_read_instant_cache git "$tmp/repo" >/dev/null 2>&1
check "a non-numeric TTL falls back to the default and ages an old entry to stale" "2" "$?"

(( failures == 0 )) || exit 1
echo "PASS: zsh env knobs"
