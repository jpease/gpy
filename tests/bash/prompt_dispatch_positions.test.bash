#!/usr/bin/env bash
# tests/bash/prompt_dispatch_positions.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/contract test for #613 (mirrors tests/fish/prompt_dispatch_positions.test.fish).
#
# Drives the real __gpy_render_prompt dispatch loop (bash/core/init.bash) with
# three stub segments and asserts each receives is_last/is_first as "true"/""
# in the correct position -- "true" for the segment at that edge, "" for
# everywhere else. Before #613 the existing "Last Segment Context" coverage in
# integration.test.bash only exercised is_last (via the "last" literal); this
# adds is_first coverage against the actual dispatch loop, not just a direct
# segment call.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-dispatch-positions.sock"

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

# Three always-on stub segments recording the raw positional args the
# dispatch loop calls them with: __gpy_segment_<name> "$is_last" "$prev_bg" "$is_first".
__gpy_segment_stuba() { printf 'stuba:%s|%s;' "$1" "$3"; }
__gpy_segment_stubb() { printf 'stubb:%s|%s;' "$1" "$3"; }
__gpy_segment_stubc() { printf 'stubc:%s|%s;' "$1" "$3"; }

__enabled_segments="stuba stubb stubc"
__gpy_render_prompt 0

check "first segment gets is_first set, is_last empty" true \
    "$([[ "$PS1" == *"stuba:|true;"* ]] && echo true || echo false)"
check "middle segment gets both empty" true \
    "$([[ "$PS1" == *"stubb:|;"* ]] && echo true || echo false)"
check "last segment gets is_last set, is_first empty" true \
    "$([[ "$PS1" == *"stubc:true|;"* ]] && echo true || echo false)"

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "PS1 was: $PS1"
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
