#!/usr/bin/env bash
# tests/bash/env_knobs_honoured.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY_IPC_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment before the integration loads (twins in tests/fish, tests/zsh).
#

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
check() {
    if [[ "$3" == "$2" ]]; then
        echo "PASS: $1"
    else
        echo "FAIL: $1 (expected [$2], got [$3])"
        failures=$((failures + 1))
    fi
}

# constants_with VAR=VALUE...: "ipc git lang" as a child bash sees them.
# shellcheck disable=SC2016 # the child shell expands its own variables
constants_with() {
    env -u GPY_IPC_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS "$@" \
        bash -c '. "$1/bash/core/constants.bash"; echo "$GPY_IPC_TIMEOUT_MS $GPY_GIT_INSTANT_CACHE_TTL_SECONDS $GPY_LANGUAGE_CACHE_TTL_SECONDS"' _ "$ROOT"
}

check "defaults when nothing is set" "150 5 30" "$(constants_with)"
check "GPY_IPC_TIMEOUT_MS is honoured" "750 5 30" "$(constants_with GPY_IPC_TIMEOUT_MS=750)"
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" "150 11 30" "$(constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11)"
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" "150 5 22" "$(constants_with GPY_LANGUAGE_CACHE_TTL_SECONDS=22)"

# shellcheck source=bash/core/ipc.bash
. bash/core/ipc.bash
secs=""
__gpy_ms_to_secs 750 secs
check "an IPC budget of 750 ms is 0.750 s" "0.750" "$secs"
__gpy_ms_to_secs fast secs
check "a non-numeric IPC budget falls back to 150 ms" "0.150" "$secs"

# Non-numeric TTL: default 5 s applies, so a 60 s-old entry is stale.
tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-knobs.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
export XDG_CACHE_HOME="$tmp/cache"
mkdir -p "$tmp/repo/.git" "$XDG_CACHE_HOME/gpy/instant-prompts"
key=""
__gpy_path_to_cache_key "$(realpath "$tmp/repo")" key
entry="$XDG_CACHE_HOME/gpy/instant-prompts/$key.git.none.bash"
printf 'cached' >"$entry"
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=soon
__gpy_read_instant_cache git "$tmp/repo" "" ignored 2>/dev/null
check "a non-numeric TTL still reads a fresh entry as fresh" "0" "$?"
touch -t 200001010000 "$entry"
__gpy_read_instant_cache git "$tmp/repo" "" ignored 2>/dev/null
status=$?
check "a non-numeric TTL falls back to the default and ages an old entry to stale" "2" "$status"

[[ "$failures" -eq 0 ]] || exit 1
echo "PASS: bash env knobs"
