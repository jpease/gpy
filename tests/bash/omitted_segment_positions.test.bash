#!/usr/bin/env bash
# tests/bash/omitted_segment_positions.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_render_prompt assigns first/last positions in the detect pass: a language
# segment that prints nothing on a cold miss must not steal its neighbour's position.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

# Supervisor off in a sandbox config: the theme export re-sets the flag.
source "$ROOT/tests/lib/supervisor_off.bash" "$TMP_ROOT"
export GPY_AGENT_SOCKET_PATH="$TMP_ROOT/missing.sock"
export GPY_LANGUAGE_ENABLED=1

source bash/gpy.bash
# The cold-miss refresh is dispatched only with the agent enabled.
export GPY_AGENT_ENABLED=1
source bash/segments/language.bash

FAILED=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        echo "  PS1 was: $PS1"
        FAILED=1
    fi
}

__gpy_add_newline=0
__gpy_request_character() { :; }
__gpy_segment_directory() { printf 'DIR[%s|%s]' "$1" "$3"; }
__gpy_segment_stubz() { printf 'Z[%s|%s]' "$1" "$3"; }
# The refresh runs in a background subshell: record it to a file.
REFRESH_LOG="$TMP_ROOT/refresh.log"
__gpy_trigger_data_refresh() { printf '%s\n' "$1" >>"$REFRESH_LOG"; }

project="$TMP_ROOT/project"
mkdir -p "$project"
touch "$project/Cargo.toml"
cd "$project" || exit 1

__enabled_segments="language directory"
__gpy_render_prompt 0
check "language cold miss: directory is first and last" true \
    "$([[ "$PS1" == *"DIR[true|true]"* ]] && echo true || echo false)"
shell_e2e_poll 1 test -s "$REFRESH_LOG" || true
check "language cold miss still requests one refresh" lang "$(cat "$REFRESH_LOG" 2>/dev/null)"

__enabled_segments="directory language stubz"
__gpy_render_prompt 0
check "omitted middle segment: positions unchanged" true \
    "$([[ "$PS1" == *"DIR[|true]Z[true|]"* ]] && echo true || echo false)"

cache_dir=""
__gpy_instant_cache_dir cache_dir
key_path=""
__gpy_instant_cache_key_path lang "$project" key_path
cache_key=""
__gpy_path_to_cache_key "$key_path" cache_key
mkdir -p "$cache_dir"
printf LANGNONE >"$cache_dir/$cache_key.lang.none.bash"
printf LANGWARM >"$cache_dir/$cache_key.lang_first.black.bash"
__enabled_segments="language directory"
__gpy_render_prompt 0
check "warm language cache: language first, directory last" true \
    "$([[ "$PS1" == *"LANGWARMDIR[true|]"* ]] && echo true || echo false)"

rm -f "$cache_dir/$cache_key".lang*
check "probe: no entry once the cache is cleared" 1 \
    "$(__gpy_instant_cache_present lang "$project"; echo $?)"
printf LANG >"$cache_dir/$cache_key.lang_first.magenta.bash"
check "probe: a contextual-only entry counts as present" 0 \
    "$(__gpy_instant_cache_present lang "$project"; echo $?)"
rm -f "$cache_dir/$cache_key".lang*
shopt -s failglob
check "probe: failglob neither errors on a miss nor stays lifted" "1|on" \
    "$(__gpy_instant_cache_present lang "$project" 2>&1; echo "$?|$(shopt -q failglob && echo on || echo off)")"
shopt -u failglob

# A key over 200 characters is stored under chunk directories; the probe looks there.
long_project="$TMP_ROOT/$(printf 'l%.0s' {1..80})/$(printf 'm%.0s' {1..80})/$(printf 'n%.0s' {1..60})"
mkdir -p "$long_project"
__gpy_instant_cache_key_path lang "$long_project" key_path
long_key=""
__gpy_path_to_cache_key "$key_path" long_key
__gpy_chunk_cache_key long_key
mkdir -p "$cache_dir/${long_key%/*}"
printf LANG >"$cache_dir/$long_key.lang_first.magenta.bash"
check "probe: a contextual-only entry under a chunked key counts as present" "chunked 0" \
    "$([[ "$long_key" == */* ]] && printf 'chunked '; __gpy_instant_cache_present lang "$long_project"; echo $?)"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
