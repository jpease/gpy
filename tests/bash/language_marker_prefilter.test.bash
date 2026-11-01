#!/usr/bin/env bash
# tests/bash/language_marker_prefilter.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The language pre-filter uses the REAL exported marker list: accepts each
# marker, rejects an empty dir, defers when no export was sourced.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

AGENT_BIN="$ROOT/gpy-agent/target/debug/gpy-agent"
if [[ ! -x "$AGENT_BIN" ]]; then
    test_skip "$AGENT_BIN not built (run \`cargo build\` in gpy-agent first)"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

detect_in() {
    (cd "$1" && __gpy_segment_language_detect)
    echo $?
}

source bash/segments/language.bash

# Unset export: defer to the request path, never a hand-kept list.
unset __gpy_lang_marker_files
mkdir "$tmp/plain"
check "unset export defers (returns 0)" 0 "$(detect_in "$tmp/plain")"

export_output="$(HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
    XDG_RUNTIME_DIR="$tmp/run" "$AGENT_BIN" theme export --shell bash 2>/dev/null)"
if [[ -z "$export_output" ]]; then
    echo "FAIL: theme export --shell bash produced no output"
    exit 1
fi
eval "$export_output"

if (( ${#__gpy_lang_marker_files[@]} == 0 )); then
    echo "FAIL: export defines no __gpy_lang_marker_files"
    exit 1
fi

check "empty dir rejected" 1 "$(detect_in "$tmp/plain")"

for marker in "${__gpy_lang_marker_files[@]}" setup.py Package.swift Rakefile; do
    dir="$tmp/m_$marker"
    mkdir -p "$dir"
    : >"$dir/$marker"
    check "marker $marker accepted" 0 "$(detect_in "$dir")"
done

mkdir -p "$tmp/m_setup.py/sub/deep"
check "marker found from a subdir" 0 "$(detect_in "$tmp/m_setup.py/sub/deep")"

test_finish "language marker prefilter"
