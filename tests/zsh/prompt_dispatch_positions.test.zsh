#!/usr/bin/env zsh
# tests/zsh/prompt_dispatch_positions.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/contract test for #613 (mirrors tests/fish/prompt_dispatch_positions.test.fish).
#
# Drives the real __gpy_render_prompt dispatch loop (zsh/core/init.zsh) with
# three stub segments and asserts each receives is_last/is_first as "true"/""
# in the correct position -- "true" for the segment at that edge, "" for
# everywhere else. Before #613 the existing "Last Segment Context" coverage in
# integration.test.zsh only exercised is_last (via the "last" literal); this
# adds is_first coverage against the actual dispatch loop, not just a direct
# segment call.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-dispatch-positions.sock"

source zsh/gpy.zsh

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
function __gpy_segment_stuba() { printf 'stuba:%s|%s;' "$1" "$3" }
function __gpy_segment_stubb() { printf 'stubb:%s|%s;' "$1" "$3" }
function __gpy_segment_stubc() { printf 'stubc:%s|%s;' "$1" "$3" }

__enabled_segments=(stuba stubb stubc)
PROMPT=$(__gpy_render_prompt 0)

check "first segment gets is_first set, is_last empty" true \
    "$([[ "$PROMPT" == *"stuba:|true;"* ]] && echo true || echo false)"
check "middle segment gets both empty" true \
    "$([[ "$PROMPT" == *"stubb:|;"* ]] && echo true || echo false)"
check "last segment gets is_last set, is_first empty" true \
    "$([[ "$PROMPT" == *"stubc:true|;"* ]] && echo true || echo false)"

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "PROMPT was: $PROMPT"
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
