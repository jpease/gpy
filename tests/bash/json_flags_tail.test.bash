#!/usr/bin/env bash
# tests/bash/json_flags_tail.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/contract test for #613.
#
# Before this issue, is_last/is_first/prev_bg JSON tail-building was
# hand-duplicated across several IPC request builders in bash/core/ipc.bash,
# and each segment re-converted the dispatch loop's is_last/is_first into
# whatever local convention it felt like using. This exercises
# __gpy_json_flags_tail directly across every (is_last, is_first, prev_bg)
# combination, plus a prev_bg that needs JSON escaping.
#
# NOTE: this table MUST stay identical to tests/fish/json_flags_tail.test.fish
# and tests/zsh/json_flags_tail.test.zsh -- the three shells' JSON tail output
# must be byte-for-byte the same for the same inputs.
#
# Must FAIL on a tree without __gpy_json_flags_tail (function undefined).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# The theme export re-sets the supervisor flag from config.toml, so disable
# it in a sandbox config; the missing socket stays there too (#836).
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-json-flags-tail.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source bash/gpy.bash

FAILED=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

# --- All 8 (is_last, is_first, prev_bg) combos, plus a 9th needing escaping ---

check "not-last, not-first, no prev_bg" \
    "" "$(__gpy_json_flags_tail "" "" "")"

check "last, not-first, no prev_bg" \
    ',"is_last":true' "$(__gpy_json_flags_tail true "" "")"

check "not-last, first, no prev_bg" \
    ',"is_first":true' "$(__gpy_json_flags_tail "" true "")"

check "last, first, no prev_bg" \
    ',"is_last":true,"is_first":true' "$(__gpy_json_flags_tail true true "")"

check "not-last, not-first, prev_bg" \
    ',"prev_bg":"blue"' "$(__gpy_json_flags_tail "" "" blue)"

check "last, not-first, prev_bg" \
    ',"is_last":true,"prev_bg":"blue"' "$(__gpy_json_flags_tail true "" blue)"

check "not-last, first, prev_bg" \
    ',"is_first":true,"prev_bg":"blue"' "$(__gpy_json_flags_tail "" true blue)"

check "last, first, prev_bg" \
    ',"is_last":true,"is_first":true,"prev_bg":"blue"' "$(__gpy_json_flags_tail true true blue)"

# 9th case: prev_bg needing escaping (embedded quote + backslash). Backslash
# is escaped first, then the quote, mirroring __gpy_escape_json's own order.
check "last, prev_bg needing escaping" \
    ',"is_last":true,"prev_bg":"blue\"bg\\path"' \
    "$(__gpy_json_flags_tail true "" 'blue"bg\path')"

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
