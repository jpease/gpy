#!/usr/bin/env fish
# Test: shell-side section renderers map a `transparent` content bg/fg to the
# terminal default instead of passing the literal keyword to `set_color`.
#
# Regression guard: a flat theme (e.g. the Starship preset) gives the shell-side
# clock segment `bg_color = "transparent"`. If the renderer forwards that string
# to `set_color -b transparent`, fish prints `set_color: Unknown color
# 'transparent'` to stderr on every prompt render. The rendered bg must resolve
# to the terminal default (SGR 49) with no stderr noise.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/renderer.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# Minimal delimiter/theme vars so the renderer runs in isolation.
set -g __segment_delim_first ""
set -g __segment_delim_start ""
set -g __segment_delim_end ""
set -g __segment_delim_last ""
set -g __prompt_open_color white
set -g __prompt_open_bg transparent
set -g __prompt_close_color white
set -g __prompt_close_bg transparent
set -g __segment_delimiter_color white
set -g __segment_delimiter_bg transparent

# --- gpy_section_standalone (used by the clock segment) ---
set -g __gpy_segment_position first
set -l err_file (mktemp)
set -l out (gpy_section_standalone transparent white "12:00" "" 2>$err_file)
set -l err (cat $err_file)

if not string match -q '*Unknown color*' -- "$err"
    check "standalone: transparent bg emits no set_color error" pass
else
    check "standalone: transparent bg emits no set_color error (stderr: $err)" fail
end

# Content bg must resolve to the terminal default, not a literal color. fish 4
# emits an explicit default bg (`…;49m`) before the text; fish 3.7 emits fg-only
# (`…m`) and leaves the bg at the prior reset. Both are correct, so assert the
# SGR immediately before the text carries no *real* (non-default) bg color
# (40-47 / 100-107 / 48;5; / 48;2;) rather than matching a version-specific code.
if string match -qr '(4[0-7]|10[0-7]|48;[25];)[0-9;]*m12:00' -- "$out"
    check "standalone: transparent bg renders as default background (got: $out)" fail
else
    check "standalone: transparent bg renders as default background" pass
end

# --- gpy_section_append (used by multi-part shell-side segments) ---
set -l err_file2 (mktemp)
gpy_section_append transparent white x 2>$err_file2 >/dev/null
set -l err2 (cat $err_file2)

if not string match -q '*Unknown color*' -- "$err2"
    check "append: transparent bg emits no set_color error" pass
else
    check "append: transparent bg emits no set_color error (stderr: $err2)" fail
end

# --- gpy_section_standalone, delimiters that borrow a transparent bg ---
# The powerline themes draw their caps in `match_bg`/`match_text`: the segment's
# OWN colors, which for a flat clock/status are `transparent`. The keyword must
# be mapped to the default before a delimiter borrows it, or set_color gets the
# literal `transparent` (#844).
set -g __prompt_open_color match_bg
set -g __prompt_close_color match_text
set -g __segment_delimiter_color match_bg
set -g __segment_delimiter_bg match_text
for position in first middle
    for last in true ""
        set -g __gpy_segment_position $position
        set -l err_file3 (mktemp)
        gpy_section_standalone transparent transparent x $last 2>$err_file3 >/dev/null
        set -l err3 (cat $err_file3)
        rm -f $err_file3
        if not string match -q '*Unknown color*' -- "$err3"
            check "standalone ($position, last='$last'): match_bg/match_text over a transparent segment emits no set_color error" pass
        else
            check "standalone ($position, last='$last'): match_bg/match_text over a transparent segment emits no set_color error (stderr: $err3)" fail
        end
    end
end

rm -f $err_file $err_file2

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count renderer transparent-bg tests passed"
