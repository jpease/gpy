#!/usr/bin/env bash
# tests/bash/color_depth_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Shell-drawn colours honour GPY_COLOR_DEPTH exactly like the agent's ColorDepth
# (rows of tests/fixtures/color_depth_vectors.tsv), including env parsing and
# the Apple_Terminal / TERM=linux downgrade; default output stays truecolor.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/renderer.bash
source bash/core/renderer.bash

FIXTURE="tests/fixtures/color_depth_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

unset TERM_PROGRAM
TERM=xterm-256color
count=0
while IFS=$'\t' read -r r g b depth fg_expected bg_expected _rest; do
    [[ -z "$r" || "$r" == '#'* ]] && continue
    count=$((count + 1))
    color=$(printf '#%02x%02x%02x' "$r" "$g" "$b")
    GPY_COLOR_DEPTH=$depth
    fg="" bg=""
    __gpy_sgr_param fg "$color" fg
    __gpy_sgr_param bg "$color" bg
    if [[ "$fg" == "$fg_expected" && "$bg" == "$bg_expected" ]]; then
        pass "vector $count [$color $depth]"
    else
        fail "vector $count [$color $depth]: expected [$fg_expected|$bg_expected], got [$fg|$bg]"
    fi
    # The same colour as a 256-index only needs the 16 mapping checked below.
done <"$FIXTURE"
[[ "$count" -gt 0 ]] || fail "no vectors read from $FIXTURE"

check() { # name expected actual
    if [[ "$2" == "$3" ]]; then pass "$1"; else fail "$1: expected [$2], got [$3]"; fi
}
depth_of() { __gpy_color_depth __d; printf '%s' "$__d"; }

# Environment parsing.
unset GPY_COLOR_DEPTH TERM_PROGRAM
TERM=xterm
check "default truecolor" truecolor "$(depth_of)"
GPY_COLOR_DEPTH=24BIT; check "24bit" truecolor "$(depth_of)"
GPY_COLOR_DEPTH=TrueColor; check "TrueColor" truecolor "$(depth_of)"
GPY_COLOR_DEPTH=" 256 "; check "trimmed 256" 256 "$(depth_of)"
GPY_COLOR_DEPTH=16; check "16" 16 "$(depth_of)"
GPY_COLOR_DEPTH=None; check "None" none "$(depth_of)"
GPY_COLOR_DEPTH=bogus; check "unknown ignored" truecolor "$(depth_of)"
GPY_COLOR_DEPTH=""; check "empty ignored" truecolor "$(depth_of)"
unset GPY_COLOR_DEPTH
TERM_PROGRAM=Apple_Terminal; check "Apple_Terminal" 256 "$(depth_of)"
unset TERM_PROGRAM
TERM=linux; check "TERM=linux" 256 "$(depth_of)"
GPY_COLOR_DEPTH=bogus; check "unknown + linux" 256 "$(depth_of)"
GPY_COLOR_DEPTH=truecolor; check "override beats linux" truecolor "$(depth_of)"
TERM=xterm

o=""
# Indexed colours: kept at 256/truecolor, mapped at 16, none at none.
GPY_COLOR_DEPTH=256; __gpy_sgr_param fg 196 o; check "index 256" "38;5;196" "$o"
GPY_COLOR_DEPTH=16; __gpy_sgr_param fg 196 o; check "index 196 -> 16" 91 "$o"
__gpy_sgr_param bg 244 o; check "index 244 bg -> 16" 100 "$o"
__gpy_sgr_param fg 3 o; check "index 3 -> 16" 33 "$o"
GPY_COLOR_DEPTH=none; __gpy_sgr_param fg red o; check "named none" "" "$o"
__gpy_sgr_pair red blue o; check "pair none" "" "$o"
GPY_COLOR_DEPTH=16; __gpy_sgr_param fg red o; check "named kept at 16" 31 "$o"

# Default output unchanged.
unset GPY_COLOR_DEPTH TERM_PROGRAM
__gpy_sgr_param fg '#5277C3' o; check "default hex" "38;2;82;119;195" "$o"

test_finish "all $count colour depth vectors + env checks"
