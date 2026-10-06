#!/usr/bin/env zsh
# tests/zsh/language_marker_prefilter.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #785: the language pre-filter iterates __gpy_lang_marker_files, the list the
# agent's `theme export` generates from marker_file_names(). This test sources
# a REAL export from the debug binary (in a temp XDG) and asserts the
# pre-filter accepts a non-git dir holding each marker (incl. setup.py,
# Package.swift, Rakefile), rejects an empty dir, and defers (returns 0) when
# the export has never been sourced.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

AGENT_BIN="$ROOT/gpy-agent/target/debug/gpy-agent"
if [[ ! -x "$AGENT_BIN" ]]; then
    test_skip "$AGENT_BIN not built (run \`cargo build\` in gpy-agent first)"
fi

FAILED=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

check() {
    local label=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

detect_in() {
    (cd "$1" && __gpy_segment_language_detect)
    echo $?
}

source zsh/segments/language.zsh

# Unset export: defer to the request path, never a hand-kept list.
unset __gpy_lang_marker_files
mkdir "$tmp/plain"
check "unset export defers (returns 0)" 0 "$(detect_in "$tmp/plain")"

export_output=$(HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
    XDG_RUNTIME_DIR="$tmp/run" "$AGENT_BIN" theme export --format zsh 2>/dev/null)
if [[ -z "$export_output" ]]; then
    echo "FAIL: theme export --format zsh produced no output"
    exit 1
fi
eval "$export_output"

if (( ${#__gpy_lang_marker_files} == 0 )); then
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

exit $FAILED
