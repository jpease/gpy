#!/usr/bin/env zsh
# tests/zsh/sgr_param_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_sgr_param maps every colour to the same SGR parameter as Bash's copy
# (and the agent's style encoder), for every row of
# tests/fixtures/sgr_param_vectors.tsv (also read by the Bash test). See the
# fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/renderer.zsh

FIXTURE="$ROOT/tests/fixtures/sgr_param_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# Colour-depth downgrade rows shared with Rust and the other shells.
CD_FIXTURE="$ROOT/tests/fixtures/color_depth_vectors.tsv"
depth_count=0
if [[ ! -r "$CD_FIXTURE" ]]; then
    fail "cannot read $CD_FIXTURE"
else
    while IFS=$'\t' read -r cr cg cb depth fg_expected bg_expected; do
        [[ -z "$cr" || "$cr" == '#'* ]] && continue
        depth_count=$((depth_count + 1))
        __GPY_COLOR_DEPTH=$depth
        printf -v hex '#%02x%02X%02x' "$cr" "$cg" "$cb"
        fg="" bg=""
        __gpy_sgr_param fg "$hex" fg
        __gpy_sgr_param bg "$hex" bg
        if [[ "$fg" == "$fg_expected" && "$bg" == "$bg_expected" ]]; then
            pass "depth vector $depth_count [$hex $depth]"
        else
            fail "depth vector $depth_count [$hex $depth]: expected [$fg_expected|$bg_expected], got [$fg|$bg]"
        fi
    done < "$CD_FIXTURE"
    [[ "$depth_count" -gt 0 ]] || fail "no depth vectors read from $CD_FIXTURE"
fi
__GPY_COLOR_DEPTH=16
__gpy_sgr_param fg 196 fg
__gpy_sgr_param bg 21 bg
[[ "$fg" == 91 && "$bg" == 44 ]] && pass "indexed colours downgrade at 16" || fail "indexed 16: [$fg|$bg]"
__GPY_COLOR_DEPTH=none
__gpy_sgr_pair red blue sgr
[[ -z "$sgr" ]] && pass "none emits no pair" || fail "none pair: [$sgr]"
__GPY_COLOR_DEPTH=truecolor

# Env detection mirrors the agent's ColorDepth::from_env.
check_detect() {
    local want=$1 got
    shift
    got=$(env -u GPY_COLOR_DEPTH -u TERM_PROGRAM -u TERM "$@" zsh -fc 'source zsh/core/renderer.zsh; print -r -- $__GPY_COLOR_DEPTH' 2>&1)
    [[ "$got" == "$want" ]] && pass "detect $want ($*)" || fail "detect $want ($*): got [$got]"
}
check_detect truecolor TERM=xterm
check_detect 256 TERM_PROGRAM=Apple_Terminal
check_detect 256 TERM=linux
check_detect 16 GPY_COLOR_DEPTH=16 TERM=linux
check_detect none GPY_COLOR_DEPTH=NONE
check_detect truecolor GPY_COLOR_DEPTH=24BIT TERM=linux
check_detect 256 GPY_COLOR_DEPTH=bogus TERM=linux
check_detect truecolor GPY_COLOR_DEPTH=bogus TERM=xterm

vector_count=0
while IFS=$'\t' read -r color fg_expected bg_expected _rest; do
    [[ -z "$color" || "$color" == '#' || "$color" == '# '* ]] && continue
    vector_count=$((vector_count + 1))
    [[ "$color" == '\e' ]] && color=""
    fg="" bg=""
    __gpy_sgr_param fg "$color" fg
    __gpy_sgr_param bg "$color" bg
    if [[ "$fg" == "$fg_expected" && "$bg" == "$bg_expected" ]]; then
        pass "vector $vector_count [$color]"
    else
        fail "vector $vector_count [$color]: expected [$fg_expected|$bg_expected], got [$fg|$bg]"
    fi
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count SGR parameter vectors"
