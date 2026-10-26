#!/usr/bin/env bash
# shellcheck disable=SC2329 # stubs are invoked by __gpy_maybe_refresh
# tests/bash/maybe_refresh_no_fork.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Segments call __gpy_maybe_refresh on every render, and almost every call is
# throttled. On bash 4.2+ a throttled call must run nothing in a subshell: a
# DEBUG trap inherited via functrace logs every command run with
# BASH_SUBSHELL > 0, i.e. inside a `$(...)` fork. The call that passes the
# throttle still dispatches. Bash 3.2 has no builtin clock, so it only checks
# that dispatch and throttling keep working.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-nofork.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_RUNTIME_DIR="$SANDBOX/runtime"
mkdir -p "$XDG_RUNTIME_DIR"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash
export GPY_AGENT_ENABLED=1

calls="$SANDBOX/refresh-calls"
forks="$SANDBOX/forks"
: >"$calls"
__gpy_trigger_data_refresh() { printf '%s\n' "$1 $2" >>"$calls"; }
__gpy_register_with_agent() { :; }
count() { wc -l <"$1" | tr -d ' '; }
count_is() { [ "$(count "$calls")" -eq "$1" ]; }
repo="$SANDBOX/my_repo dir:x"
# The first call passes the throttle and dispatches; ten more inside the
# 500 ms window are throttled and are the ones measured.
__gpy_maybe_refresh git "$repo" git_last true black ""
: >"$forks"
saved_debug_trap="$(trap -p DEBUG)"
set -T
trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "$BASH_COMMAND" >>"$forks"' DEBUG
for ((i = 0; i < 10; i++)); do
    __gpy_maybe_refresh git "$repo" git_last true black ""
done
trap - DEBUG
set +T
eval "$saved_debug_trap"

shell_e2e_poll 2 count_is 1 || true
sleep 0.3 # window in which a throttle-violating dispatch would land
expect_eq "one dispatch for eleven calls inside the window" "$(count "$calls")" "1"

if [[ -n "${EPOCHREALTIME:-}" || "${__gpy_have_printf_epoch:-0}" == "1" ]]; then
    expect_eq "bash $BASH_VERSION: throttled calls run nothing in a subshell" \
        "$(count "$forks")" "0"
fi

# Past the window the same bucket dispatches again; a whole second, as
# bash 3.2 reads the clock in seconds.
sleep 1
__gpy_maybe_refresh git "$repo" git_last true black ""
shell_e2e_poll 2 count_is 2 || true
expect_eq "a call after the window dispatches" "$(count "$calls")" "2"

test_finish "throttled __gpy_maybe_refresh forks nothing"
