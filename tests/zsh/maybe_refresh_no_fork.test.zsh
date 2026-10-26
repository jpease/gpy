#!/usr/bin/env zsh
# tests/zsh/maybe_refresh_no_fork.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Mirrors tests/bash/maybe_refresh_no_fork.test.bash. A throttled
# __gpy_maybe_refresh must run nothing in a subshell: a DEBUG trap, which
# `$(...)` inherits, logs every command run with ZSH_SUBSHELL > 0. The call
# that passes the throttle still dispatches.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-nofork.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_RUNTIME_DIR="$SANDBOX/runtime"
mkdir -p "$XDG_RUNTIME_DIR"

source zsh/gpy.zsh
export GPY_AGENT_ENABLED=1
calls="$SANDBOX/refresh-calls"
forks="$SANDBOX/forks"
: >"$calls"
: >"$forks"
__gpy_trigger_data_refresh() { print -r -- "$1 $2" >>"$calls"; }
__gpy_register_with_agent() { :; }
repo="$SANDBOX/my_repo dir:x"
# The first call passes the throttle and dispatches; ten more inside the
# 500 ms window are throttled and are the ones measured.
__gpy_maybe_refresh git "$repo" git_last true black ""
trap '(( ZSH_SUBSHELL > 0 )) && print -r -- "$ZSH_DEBUG_CMD" >>"$forks"' DEBUG
for (( i = 0; i < 10; i++ )); do
    __gpy_maybe_refresh git "$repo" git_last true black ""
done
trap - DEBUG

shell_e2e_poll 2 shell_e2e_has_lines "$calls" 1 || true
sleep "$SHELL_E2E_ABSENCE_WINDOW" # window in which a throttle-violating dispatch would land
expect_eq "one dispatch for eleven calls inside the window" "$(shell_e2e_count_lines "$calls")" "1"
expect_eq "zsh $ZSH_VERSION: throttled calls run nothing in a subshell" "$(shell_e2e_count_lines "$forks")" "0"

# Past the window the same bucket dispatches again.
sleep 0.6
__gpy_maybe_refresh git "$repo" git_last true black ""
shell_e2e_poll 2 shell_e2e_has_lines "$calls" 2 || true
expect_eq "a call after the window dispatches" "$(shell_e2e_count_lines "$calls")" "2"

test_finish "throttled __gpy_maybe_refresh forks nothing"
