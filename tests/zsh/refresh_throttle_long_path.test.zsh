#!/usr/bin/env zsh
# tests/zsh/refresh_throttle_long_path.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Mirrors tests/bash/refresh_throttle_long_path.test.bash.
#
# __gpy_maybe_refresh keeps its one-per-500 ms bucket in a stamp file whose
# name must fit a filename component however long the project path is:
#   (a) two immediate refreshes for a short path dispatch once (control);
#   (b) the same for long ASCII and long non-ASCII/escaped paths, silently;
#   (c) distinct long contexts and variant tokens keep their own buckets;
#   (d) after the interval the long-path bucket dispatches again;
#   (e) a stamp of another bucket under the same name does not throttle;
#   (f) a stamp that cannot be written skips the refresh instead of
#       dispatching one per render, and prints nothing.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-throttle.XXXXXX")
trap 'rm -rf "$SANDBOX"' EXIT

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export XDG_RUNTIME_DIR="$SANDBOX/runtime"
mkdir -p "$XDG_RUNTIME_DIR"

source zsh/gpy.zsh
export GPY_AGENT_ENABLED=1
calls="$SANDBOX/refresh-calls"
: >"$calls"
: >"$SANDBOX/stderr"
# Users may set NO_CLOBBER; stamps are rewritten every interval regardless.
# (It also stops `>>` creating files, hence the logs above exist first.)
setopt no_clobber
__gpy_trigger_data_refresh() { print -r -- "$1 $2" >>"$calls"; }
__gpy_register_with_agent() { :; }
count() { wc -l <"$calls" | tr -d ' '; }
count_is() { [ "$(count)" -eq "$1" ]; }
# A fixed clock: back-to-back calls are always inside the 500 ms window.
fake_ms=1000000
__gpy_now_ms() { print -r -- "$fake_ms"; }

# expect_dispatches LABEL N: N refreshes so far, and no more land in the window.
expect_dispatches() {
    shell_e2e_poll 2 count_is "$2" || true
    sleep 0.3 # window in which an extra (throttle-violating) dispatch would land
    if count_is "$2"; then
        pass "$1"
    else
        fail "$1: $(count) dispatches, want $2"
    fi
}

# twice ROOT [TOKEN]: two immediate refreshes of one bucket, the second from a
# subshell as segment renders are; stderr is collected.
twice() {
    __gpy_maybe_refresh lang "$1" lang "" "" "" "${2:-}" 2>>"$SANDBOX/stderr"
    (__gpy_maybe_refresh lang "$1" lang "" "" "" "${2:-}" 2>>"$SANDBOX/stderr")
}

long_ascii="$SANDBOX/${(l:80::a:)}/${(l:80::b:)}/${(l:80::c:)}"
unicode_part= escaped_part=
repeat 12 unicode_part+='проект_日本 '
repeat 30 escaped_part+='x:y\z_'
long_unicode="$SANDBOX/$unicode_part/$escaped_part"
long_sibling="${long_ascii%c}d"

twice "$SANDBOX/short"
expect_dispatches "short path: two immediate refreshes dispatch once" 1
twice "$long_ascii"
expect_dispatches "long ASCII path: two immediate refreshes dispatch once" 2
twice "$long_unicode"
expect_dispatches "long non-ASCII/escaped path: two immediate refreshes dispatch once" 3
twice "$long_sibling"
expect_dispatches "a long path differing only in its last byte has its own bucket" 4
twice "$long_ascii" 231
expect_dispatches "a variant token on a long path has its own bucket" 5
if [[ -s "$SANDBOX/stderr" ]]; then
    fail "refreshes printed to stderr: $(<"$SANDBOX/stderr")"
else
    pass "no refresh printed anything"
fi

fake_ms=$((fake_ms + 600))
twice "$long_ascii"
expect_dispatches "after the interval the long-path bucket dispatches once more" 6

# A stamp holding another bucket under the same name (a hash collision) does
# not throttle this one. A fresh runtime root holds only this bucket's stamp.
export XDG_RUNTIME_DIR="$SANDBOX/runtime-collision"
mkdir -p "$XDG_RUNTIME_DIR"
__gpy_maybe_refresh lang "$long_unicode" lang "" "" ""
expect_dispatches "a fresh runtime root starts with no stamp" 7
stamps=("$XDG_RUNTIME_DIR"/gpy/refresh-throttle/*(N))
print -rn -- "$fake_ms lang.lang..another-bucket" >|"${stamps[1]}"
twice "$long_unicode"
expect_dispatches "a stamp of another bucket under the same name does not throttle" 8

# The memoized name follows the bucket: alternating two long buckets yields
# each one's own name, the same as computed with no memo.
bucket_a="lang.lang..$(__gpy_path_to_cache_key "$long_ascii")"
bucket_b="lang.lang..$(__gpy_path_to_cache_key "$long_unicode")"
__gpy_refresh_stamp_name "$bucket_a" name_a1
__gpy_refresh_stamp_name "$bucket_b" name_b
__gpy_refresh_stamp_name "$bucket_a" name_a2
fresh_b="$(__gpy_refresh_stamp_memo_bucket="" && __gpy_refresh_stamp_name "$bucket_b" n && print -rn -- "$n")"
if [[ "$name_a1" == "$name_a2" && "$name_a1" != "$name_b" && "$name_b" == "$fresh_b" ]]; then
    pass "alternating long buckets get their own memoized names"
else
    fail "memoized names: a1=$name_a1 b=$name_b a2=$name_a2 fresh b=$fresh_b"
fi

# The runtime root is a file, so no stamp can be persisted.
fake_ms=$((fake_ms + 600))
export XDG_RUNTIME_DIR="$SANDBOX/runtime-blocked"
mkdir -p "$XDG_RUNTIME_DIR"
: >"$XDG_RUNTIME_DIR/gpy"
: >|"$SANDBOX/stderr"
twice "$long_ascii"
twice "$SANDBOX/short"
expect_dispatches "an unwritable stamp skips the refresh rather than flooding" 8
if [[ -s "$SANDBOX/stderr" ]]; then
    fail "an unwritable stamp printed to stderr: $(<"$SANDBOX/stderr")"
else
    pass "an unwritable stamp prints nothing"
fi

test_finish "the refresh throttle holds for long and unwritable stamp paths"
