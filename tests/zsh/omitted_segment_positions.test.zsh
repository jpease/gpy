#!/usr/bin/env zsh
# tests/zsh/omitted_segment_positions.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #766 (mirrors tests/fish/omitted_segment_positions.test.fish).
#
# __gpy_render_prompt assigns first/last positions from the detect pass,
# before anything renders. The language segment passes detect on a project
# marker but prints nothing on a cold miss (no instant-cache entry), so its
# neighbour lost its first-segment form or kept a trailing separator. Drives
# the real dispatch loop with no agent socket, an empty instant cache and a
# directory stub that prints the is_last/is_first it was called with.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$TMP_ROOT/missing.sock"
export XDG_CACHE_HOME="$TMP_ROOT/cache"
export GPY_LANGUAGE_ENABLED=1

source zsh/gpy.zsh
# Agent-free sandbox config (above) keeps sourcing from starting a daemon; the
# refresh the cold miss still requests is dispatched only with the agent enabled.
export GPY_AGENT_ENABLED=1
source zsh/segments/language.zsh

FAILED=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        echo "  PROMPT was: $PROMPT"
        FAILED=1
    fi
}

__gpy_add_newline=0
function __gpy_request_character() { : }
function __gpy_segment_directory() { printf 'DIR[%s|%s]' "$1" "$3" }
function __gpy_segment_stubz() { printf 'Z[%s|%s]' "$1" "$3" }
# The refresh runs in a background subshell: record it to a file.
REFRESH_LOG="$TMP_ROOT/refresh.log"
function __gpy_trigger_data_refresh() { printf '%s\n' "$1" >>"$REFRESH_LOG" }

project="$TMP_ROOT/project"
mkdir -p "$project"
touch "$project/Cargo.toml"
cd "$project" || exit 1

# --- Language cold miss first: directory opens and closes the line ---
__enabled_segments=(language directory)
PROMPT=$(__gpy_render_prompt 0)
check "language cold miss: directory is first and last" true \
    "$([[ "$PROMPT" == *"DIR[true|true]"* ]] && echo true || echo false)"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    [[ -s "$REFRESH_LOG" ]] && break
    sleep 0.1
done
check "language cold miss still requests one refresh" lang "$(cat "$REFRESH_LOG" 2>/dev/null)"

# --- Language cold miss in the middle: neighbours keep their positions ---
__enabled_segments=(directory language stubz)
PROMPT=$(__gpy_render_prompt 0)
check "omitted middle segment: positions unchanged" true \
    "$([[ "$PROMPT" == *"DIR[|true]Z[true|]"* ]] && echo true || echo false)"

# --- Warm cache: language renders and keeps the first position ---
cache_dir=$(__gpy_instant_cache_dir)
__gpy_instant_cache_key_path lang "$project" key_path
cache_key=$(__gpy_path_to_cache_key "$key_path")
mkdir -p "$cache_dir"
printf LANGNONE >"$cache_dir/$cache_key.lang.none.zsh"
printf LANGWARM >"$cache_dir/$cache_key.lang_first.black.zsh"
__enabled_segments=(language directory)
PROMPT=$(__gpy_render_prompt 0)
check "warm language cache: language first, directory last" true \
    "$([[ "$PROMPT" == *"LANGWARMDIR[true|]"* ]] && echo true || echo false)"

# --- The presence probe counts contextual-only entries (no `.none`) ---
rm -f "$cache_dir/$cache_key".lang*
check "probe: no entry once the cache is cleared" 1 \
    "$(__gpy_instant_cache_present lang "$project"; echo $?)"
printf LANG >"$cache_dir/$cache_key.lang_first.magenta.zsh"
check "probe: a contextual-only entry counts as present" 0 \
    "$(__gpy_instant_cache_present lang "$project"; echo $?)"
rm -f "$cache_dir/$cache_key".lang*
# NOMATCH is on by default; an unmatched probe must not print an error.
check "probe: a miss prints no NOMATCH error" "1" \
    "$(__gpy_instant_cache_present lang "$project" 2>&1; echo $?)"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
