#!/usr/bin/env zsh
# tests/zsh/prompt_dispatch_positions.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Contract test for segment positions (mirrors tests/fish/prompt_dispatch_positions.test.fish).
#
# Drives the real __gpy_render_prompt dispatch loop (zsh/core/init.zsh) with
# three stub segments and asserts each receives is_last/is_first as "true"/""
# in the correct position -- "true" for the segment at that edge, "" for
# everywhere else. This covers is_first as well as is_last against the actual
# dispatch loop, not just a direct segment call.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-dispatch-positions.sock"

source zsh/gpy.zsh


# Three always-on stub segments recording the raw positional args the
# dispatch loop calls them with: __gpy_segment_<name> "$is_last" "$prev_bg" "$is_first".
function __gpy_segment_stuba() { printf 'stuba:%s|%s;' "$1" "$3" }
function __gpy_segment_stubb() { printf 'stubb:%s|%s;' "$1" "$3" }
function __gpy_segment_stubc() { printf 'stubc:%s|%s;' "$1" "$3" }

__enabled_segments=(stuba stubb stubc)
__gpy_render_prompt 0; PROMPT=$__gpy_render_out

check "first segment gets is_first set, is_last empty" true \
    "$([[ "$PROMPT" == *"stuba:|true;"* ]] && echo true || echo false)"
check "middle segment gets both empty" true \
    "$([[ "$PROMPT" == *"stubb:|;"* ]] && echo true || echo false)"
check "last segment gets is_last set, is_first empty" true \
    "$([[ "$PROMPT" == *"stubc:true|;"* ]] && echo true || echo false)"

test_finish "prompt dispatch positions"
