#!/usr/bin/env bash
# tests/bash/instant_cache_read_no_fork.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A cache-hit render reads the instant cache on every prompt, and must not
# fork beyond the external reads bash has no builtin for: a DEBUG trap
# inherited via functrace logs every command run with BASH_SUBSHELL > 0 while
# __gpy_read_instant_cache and the git and language segments serve hits. Only
# `stat` (the entry's age) and `date` (the clock on bash without a builtin
# epoch) may run there; `realpath` is memoized per directory by __gpy_resolve_pwd.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-icnofork.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_CACHE_HOME="$SANDBOX/cache"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

# expect_eq LABEL GOT WANT
expect_eq() { if [[ "$2" == "$3" ]]; then pass "$1"; else fail "$1: got $2, want $3"; fi; }

repo="$SANDBOX/repo"
mkdir -p "$repo/.git"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts" key=""
__gpy_path_to_cache_key "$(realpath "$repo")" key
mkdir -p "$cache_dir"
printf 'HIT' >"$cache_dir/$key.git_first_last._5277C3.bash"
printf 'LANG' >"$cache_dir/$key.lang_first_last._5277C3.bash"

forks="$SANDBOX/forks"
out="$SANDBOX/out"
: >"$forks"
saved_debug_trap="$(trap -p DEBUG)"
set -T
text=""
cd "$repo" || exit 1
__gpy_resolve_pwd
trap '(( BASH_SUBSHELL > 0 )) && [[ "$BASH_COMMAND" != stat\ * && "$BASH_COMMAND" != date\ * ]] && printf "%s\n" "${FUNCNAME[0]}: $BASH_COMMAND" >>"$forks"' DEBUG
__gpy_read_instant_cache git_first_last "$PWD" "#5277C3" text
status=$?
__gpy_segment_git true "#5277C3" true >"$out"
__gpy_segment_language true "#5277C3" true >>"$out"
trap - DEBUG
cd "$ROOT" || exit 1
set +T
eval "$saved_debug_trap"

expect_eq "cache hit served" "$status:$text" "0:HIT"
expect_eq "segments render their hits" "$(<"$out")" "HITLANG"
expect_eq "bash $BASH_VERSION: no subshell beyond stat/date" "$(<"$forks")" ""

test_finish "a cache-hit render forks only for stat/date"
