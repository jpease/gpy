#!/usr/bin/env bash
# tests/bash/segment_bg_export.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# __gpy_segment_bg comes from the debug agent's `theme export`: it returns the
# git bg, and no hand-copied `case "$segment" in` table remains.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

AGENT_BIN="$ROOT/gpy-agent/target/debug/gpy-agent"
if [[ ! -x "$AGENT_BIN" ]]; then
    test_skip "$AGENT_BIN not built (run \`cargo build\` in gpy-agent first)"
fi

source bash/core/constants.bash
source bash/core/ipc.bash

export_output="$("$AGENT_BIN" theme export --format bash 2>/dev/null)"
if [[ -z "$export_output" ]]; then
    echo "FAIL: gpy-agent theme export --format bash produced no output"
    exit 1
fi
eval "$export_output"

check "__gpy_segment_bg git matches \$__color_git_clean_bg" \
    "${__color_git_clean_bg:-}" "$(__gpy_segment_bg git)"

# Fork-free out-var calling convention (the render loop's hot path).
out_var_result=""
__gpy_segment_bg git out_var_result
check "__gpy_segment_bg git OUT_VAR writes the same value" \
    "${__color_git_clean_bg:-}" "$out_var_result"

check "unknown segment returns empty" "" "$(__gpy_segment_bg not-a-real-segment)"

# shellcheck disable=SC2016 # intentional literal grep pattern, not a shell expansion
hit_count="$(grep -rl 'case "\$segment" in' bash 2>/dev/null | wc -l | tr -d ' ')"
check "no 'case \"\$segment\" in' table left under bash/" 0 "$hit_count"

test_finish "segment_bg export test"
