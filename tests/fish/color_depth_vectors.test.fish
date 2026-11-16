#!/usr/bin/env fish
# tests/fish/color_depth_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The pills Fish draws itself honour GPY_COLOR_DEPTH like the agent's
# ColorDepth (gpy-agent/src/formatter/style_encoder.rs): every row of
# tests/fixtures/color_depth_vectors.tsv (also read by the Rust, Bash and Zsh
# tests), the environment parsing and the Apple Terminal / TERM=linux
# downgrade, and byte-identical set_color output at the default depth.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l repo_root (path resolve (path dirname (status --filename))/../..)
cd "$repo_root"

source fish/core/renderer.fish
source fish/functions/fish_prompt.fish

print_test_header "colour depth vectors"

set -l fixture "$repo_root/tests/fixtures/color_depth_vectors.tsv"
if not test -r "$fixture"
    fail "cannot read $fixture"
    test_finish "colour depth vectors"
end
set -l vector_count 0
while read -l line
    string match -q -- '#*' "$line"; and continue
    test -z "$line"; and continue
    set -l cols (string split \t -- "$line")
    set vector_count (math $vector_count + 1)
    set -g __gpy_color_depth $cols[4]
    set -l lower (printf '#%02x%02x%02x' $cols[1..3])
    set -l upper (printf '%02X%02X%02X' $cols[1..3])
    set -l fg (__gpy_sgr_param fg $lower)
    set -l bg (__gpy_sgr_param bg $upper)
    check "vector $vector_count [$lower $cols[4]]" "$cols[5]|$cols[6]" "$fg|$bg"
end <$fixture
test $vector_count -gt 0; or fail "no vectors read from $fixture"

print_test_header environment

function depth_of
    command env -u GPY_COLOR_DEPTH -u TERM_PROGRAM -u TERM $argv (status fish-path) --no-config -c 'source fish/core/renderer.fish; printf "[%s]" "$__gpy_color_depth"'
end
check "default: set_color draws" "[]" (depth_of TERM=xterm-256color)
check "Apple Terminal: 256" "[256]" (depth_of TERM_PROGRAM=Apple_Terminal)
check "TERM=linux: 256" "[256]" (depth_of TERM=linux)
check "24BIT is truecolor and beats TERM=linux" "[truecolor]" (depth_of GPY_COLOR_DEPTH=24BIT TERM=linux)
check "TrueColor is truecolor" "[truecolor]" (depth_of GPY_COLOR_DEPTH=TrueColor TERM=xterm)
check "blanks around 16 are ignored" "[16]" (depth_of "GPY_COLOR_DEPTH= 16 " TERM=xterm)
check "NONE is none" "[none]" (depth_of GPY_COLOR_DEPTH=NONE)
check "an unknown value is ignored" "[]" (depth_of GPY_COLOR_DEPTH=bogus TERM=xterm)
check "an unknown value keeps the TERM=linux downgrade" "[256]" (depth_of GPY_COLOR_DEPTH=bogus TERM=linux)
check "an empty value is ignored" "[]" (depth_of GPY_COLOR_DEPTH= TERM=xterm)

print_test_header "drawn pills"

function bytes
    $argv | od -An -c | string join '' | string replace -ra '\s+' ' ' | string trim
end

# Default depth: exactly what set_color writes.
set -g __gpy_color_depth ""
set -l same 1
for pair in "#5277c3 brblack" "red blue" "normal normal" "#d79966 #2f2f2f" "brmagenta white"
    set -l c (string split ' ' -- $pair)
    if test "$(bytes __gpy_set_color -b $c[1] $c[2])" != "$(bytes set_color -b $c[1] $c[2])"
        set same 0
        fail "default depth: __gpy_set_color -b $pair matches set_color"
    end
end
test $same = 1; and pass "default depth: __gpy_set_color writes set_color's bytes"
check "default depth: the status colour is set_color's" "$(bytes set_color green)" "$(bytes __gpy_set_fg_color green)"

set -g __gpy_color_depth 256
check "256: one SGR, foreground then background" "$(bytes printf '\e[90;48;5;68m')" "$(bytes __gpy_set_color -b '#5277c3' brblack)"
check "256: the prompt character colour" "$(bytes printf '\e[38;5;173m')" "$(bytes __gpy_set_fg_color '#d79966')"
check "256: set_color's short hex spellings" "38;5;196|38;5;196" (__gpy_sgr_param fg ff0000)"|"(__gpy_sgr_param fg '#f00')
check "256: normal is the terminal default" "39|49" (__gpy_sgr_param fg normal)"|"(__gpy_sgr_param bg normal)

set -g __gpy_color_depth 16
set -g __gpy_segment_position first
set -l pill (gpy_section_standalone '#5277c3' '#d79966' x true | string collect)
if string match -q -- '*;2;*' "$pill"; or string match -q -- '*;5;*' "$pill"
    fail "16: a pill holds no 24-bit or 256 code" (string escape -- "$pill")
else if string match -q -- '*104m*' "$pill"
    pass "16: a pill draws the nearest of the 16 colours"
else
    fail "16: a pill draws the nearest of the 16 colours" (string escape -- "$pill")
end

set -g __gpy_color_depth none
check "none: no colour code" "" "$(__gpy_set_color -b red '#5277c3')"
check "none: no prompt character colour" "" "$(__gpy_set_fg_color red)"

# The TERM=linux downgrade reaches a whole prompt pill in a fresh shell.
set -l linux_pill (command env -u GPY_COLOR_DEPTH -u TERM_PROGRAM TERM=linux (status fish-path) --no-config -c 'source fish/core/renderer.fish; gpy_section_standalone "#5277c3" white x true' | string collect)
if string match -q -- '*48;5;68*' "$linux_pill"
    pass "TERM=linux: a pill background is drawn from the 256 palette"
else
    fail "TERM=linux: a pill background is drawn from the 256 palette" (string escape -- "$linux_pill")
end

test_finish "colour depth: $vector_count vectors"
