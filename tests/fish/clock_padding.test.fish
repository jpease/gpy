#!/usr/bin/env fish
# Test: the shell-side Fish clock segment pads its rendered time with a
# leading space (matching the other segments' `[ $content]` leading space)
# and relies on the renderer for the trailing gap, instead of padding both
# sides itself.
#
# Regression guard: in a flat theme (e.g. the Starship preset) every segment
# delimiter is empty, so segments would abut without some separator.
# gpy_section_standalone/gpy_section_end now emit a same-background gap space
# before a non-last closing cap unconditionally (added for the triangle-cap
# default theme), which covers this case; the clock must NOT also add its own
# trailing pad, or non-last renders would carry two spaces instead of one.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/renderer.fish"
source "$repo_root/fish/segments/clock.fish"

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

# Deterministic 12-hour clock config.
set -g __time_format 12
set -g __clock_show_leading_zero 0
set -g __clock_show_seconds 0
set -g __color_clock_bg transparent
set -g __color_clock_fg white

# --- End-to-end (real renderer): flat theme, not-last, must still separate ---
# Flat-theme delimiters: empty, mirroring the Starship preset.
set -g __gpy_segment_position first
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

set -l not_last_raw (segment_clock_render "" | string collect)
set -l not_last_stripped (string replace -ra '\x1b\[[0-9;]*m' '' -- "$not_last_raw")

if string match -qr ' $' -- "$not_last_stripped"; and not string match -qr '  $' -- "$not_last_stripped"
    check "flat theme, not-last: exactly one trailing separator space" pass
else
    check "flat theme, not-last: exactly one trailing separator space (got: '$not_last_stripped')" fail
end

set -l last_raw (segment_clock_render last | string collect)
set -l last_stripped (string replace -ra '\x1b\[[0-9;]*m' '' -- "$last_raw")

if string match -qr ' $' -- "$last_stripped"
    check "flat theme, is-last: no trailing separator space (got: '$last_stripped')" fail
else
    check "flat theme, is-last: no trailing separator space" pass
end

# --- Content shape: capture the content the clock hands to the renderer
# without ANSI noise by stubbing the renderer helper to record its content
# argument (must run after the end-to-end checks above, which need the real
# gpy_section_standalone).
function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_clock_content "$content"
end

segment_clock_render last

if string match -q ' *' -- "$__test_clock_content"
    check "clock content has a leading space" pass
else
    check "clock content has a leading space (got: '$__test_clock_content')" fail
end

if string match -q '* ' -- "$__test_clock_content"
    check "clock content has no trailing space (got: '$__test_clock_content')" fail
else
    check "clock content has no trailing space (the renderer supplies the gap instead)" pass
end

if string match -qr ':[0-9][0-9]' -- "$__test_clock_content"
    check "clock content still contains the rendered time" pass
else
    check "clock content still contains the rendered time (got: '$__test_clock_content')" fail
end

# --- Date format strings (#644): the exact strftime spec for each clock mode.
# The prompt folds this into its single `date` call, so a wrong spec here is
# a wrong clock for every render. It is also the spec the agent embeds in a
# templated clock (ClockResolver::time_spec, and Bash/Zsh's copies): when the
# agent renders the clock (#844) fish finds this exact text in the response, so
# the no-pad forms are `%-I`/`%-H`, not the blank-padded `%l`/`%k`.
function check_format --argument-names label time_format leading_zero seconds expected
    set -g __time_format $time_format
    set -g __clock_show_leading_zero $leading_zero
    set -g __clock_show_seconds $seconds
    set -l actual (__gpy_clock_date_format)
    if test "$actual" = "$expected"
        check "$label -> '$expected'" pass
    else
        check "$label -> expected '$expected', got '$actual'" fail
    end
end

check_format "12h, no leading zero, no seconds" 12 0 0 '%-I:%M %p'
check_format "12h, leading zero" 12 1 0 '%I:%M %p'
check_format "24h, leading zero" 24 1 0 '%H:%M'
check_format "24h, bare hour" 24 0 0 '%-H:%M'
check_format "24h, leading zero, seconds" 24 1 1 '%H:%M:%S'
check_format "12h, seconds keep AM/PM after the seconds" 12 0 1 '%-I:%M:%S %p'

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count clock padding tests passed"
