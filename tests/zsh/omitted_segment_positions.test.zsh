#!/usr/bin/env zsh
# tests/zsh/omitted_segment_positions.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Mirrors tests/fish/omitted_segment_positions.test.fish.
#
# __gpy_render_prompt assigns first/last positions from the detect pass,
# before anything renders. The language segment passes detect on a project
# marker but prints nothing on a cold miss (no instant-cache entry), so the
# positions must account for it: its neighbour keeps its first-segment form
# and no trailing separator is left behind. Drives
# the real dispatch loop with no agent socket, an empty instant cache and a
# directory stub that prints the is_last/is_first it was called with.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$TMP_ROOT/missing.sock"
export XDG_CACHE_HOME="$TMP_ROOT/cache"

source zsh/gpy.zsh
# Agent-free sandbox config (above) keeps sourcing from starting a daemon; the
# refresh the cold miss still requests is dispatched only with the agent enabled.
export GPY_AGENT_ENABLED=1
source zsh/segments/language.zsh


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
__gpy_render_prompt 0; PROMPT=$__gpy_render_out
check "language cold miss: directory is first and last" true \
    "$([[ "$PROMPT" == *"DIR[true|true]"* ]] && echo true || echo false)"
refresh_logged() { [[ -s "$REFRESH_LOG" ]]; }
shell_e2e_poll 1 refresh_logged
check "language cold miss still requests one refresh" lang "$(cat "$REFRESH_LOG" 2>/dev/null)"

# --- Language cold miss in the middle: neighbours keep their positions ---
__enabled_segments=(directory language stubz)
__gpy_render_prompt 0; PROMPT=$__gpy_render_out
check "omitted middle segment: positions unchanged" true \
    "$([[ "$PROMPT" == *"DIR[|true]Z[true|]"* ]] && echo true || echo false)"

# --- Warm cache: language renders and keeps the first position ---
__gpy_instant_cache_dir cache_dir
__gpy_instant_cache_key_path lang "$project" key_path
__gpy_path_to_cache_key "$key_path" cache_key
mkdir -p "$cache_dir"
printf LANGNONE >"$cache_dir/$cache_key.lang.none.zsh"
printf LANGWARM >"$cache_dir/$cache_key.lang_first.black.zsh"
__enabled_segments=(language directory)
__gpy_render_prompt 0; PROMPT=$__gpy_render_out
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

# --- A key over 200 characters is stored under chunk directories; the probe looks there ---
long_project="$TMP_ROOT/${(l:80::l:)}/${(l:80::m:)}/${(l:60::n:)}"
mkdir -p "$long_project"
__gpy_instant_cache_key_path lang "$long_project" key_path
__gpy_path_to_cache_key "$key_path" long_key
__gpy_chunk_cache_key long_key
mkdir -p "$cache_dir/${long_key%/*}"
printf LANG >"$cache_dir/$long_key.lang_first.magenta.zsh"
check "probe: a contextual-only entry under a chunked key counts as present" "chunked 0" \
    "$([[ "$long_key" == */* ]] && printf 'chunked '; __gpy_instant_cache_present lang "$long_project"; echo $?)"

test_finish "omitted segment positions"
