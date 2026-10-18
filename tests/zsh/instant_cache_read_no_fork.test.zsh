#!/usr/bin/env zsh
# tests/zsh/instant_cache_read_no_fork.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Mirrors tests/bash/instant_cache_read_no_fork.test.bash. While
# __gpy_read_instant_cache and the git and language segments serve hits, a
# DEBUG trap logs every command run with ZSH_SUBSHELL > 0. Zsh resolves
# symlinks (`:A`) and mtimes (zstat) in-process, so nothing may run there.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-icnofork.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_CACHE_HOME="$SANDBOX/cache"

source zsh/gpy.zsh

# expect_eq LABEL GOT WANT
expect_eq() { if [[ "$2" == "$3" ]]; then pass "$1"; else fail "$1: got $2, want $3"; fi; }

repo="$SANDBOX/repo"
mkdir -p "$repo/.git"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts" key=""
__gpy_path_to_cache_key "$(realpath "$repo")" key
mkdir -p "$cache_dir"
printf 'HIT' >"$cache_dir/$key.git_first_last._5277C3.zsh"
printf 'LANG' >"$cache_dir/$key.lang_first_last._5277C3.zsh"

forks="$SANDBOX/forks"
out="$SANDBOX/out"
: >"$forks"
text=""
cd "$repo" || exit 1
trap '(( ZSH_SUBSHELL > 0 )) && print -r -- "${funcstack[1]}: $ZSH_DEBUG_CMD" >>"$forks"' DEBUG
__gpy_read_instant_cache git_first_last "$repo" "#5277C3" text
cache_status=$?
__gpy_segment_git true "#5277C3" true >"$out"
__gpy_segment_language true "#5277C3" true >>"$out"
trap - DEBUG
cd "$ROOT" || exit 1

expect_eq "cache hit served" "$cache_status:$text" "0:HIT"
expect_eq "segments render their hits" "$(<"$out")" "HITLANG"
expect_eq "zsh $ZSH_VERSION: no subshell at all" "$(<"$forks")" ""

test_finish "a cache-hit render forks nothing"
