#!/usr/bin/env bash
# tests/bash/prompt_dispatch_positions.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Drives the real __gpy_render_prompt loop with stub segments; is_last/is_first
# must be "true" at their edge and "" elsewhere.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# Sandbox config disables the supervisor (theme export reads config.toml).
SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-dispatch-positions.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

source bash/gpy.bash


# Stubs record args: __gpy_segment_<name> "$is_last" "$prev_bg" "$is_first".
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

test_finish "prompt dispatch positions"
