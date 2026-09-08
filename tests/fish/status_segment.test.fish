#!/usr/bin/env fish
# Test: status segment (fish/segments/status.fish) — powerline chevron
# tracking. `__gpy_last_segment_bg` must reflect the background the status
# pill actually rendered (ok vs fail branch, each with its own color and its
# own literal fallback) so the NEXT segment's opening chevron blends
# correctly. Regression for #453 (status never updated the tracker at all).
#
# Sources the segment file directly and stubs gpy_section_standalone, mirroring
# hostname_segment.test.fish / username_segment.test.fish.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/segments/status.fish"

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

function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_status_bg "$bg"
    set -g __test_status_fg "$fg"
    set -g __test_status_content "$content"
end

# --- successful (status 0) path, theme colors configured ---

set -g __gpy_last_status 0
set -g __color_status_ok_bg cyan
set -g __color_status_ok_fg black
set -g __icon_status_ok OK
set -g __gpy_last_segment_bg magenta

segment_status_render last

if test "$__gpy_last_segment_bg" = cyan
    check "ok path: tracks __gpy_last_segment_bg to the configured ok background" pass
else
    check "ok path: tracks __gpy_last_segment_bg to the configured ok background (got: '$__gpy_last_segment_bg')" fail
end

if test "$__gpy_last_segment_bg" = "$__test_status_bg"
    check "ok path: tracker matches the bg actually passed to gpy_section_standalone" pass
else
    check "ok path: tracker matches the bg actually passed to gpy_section_standalone (got: '$__gpy_last_segment_bg' vs '$__test_status_bg')" fail
end

# --- failed (non-zero status) path, theme colors configured ---

set -g __gpy_last_status 1
set -g __color_status_fail_bg magenta
set -g __color_status_fail_fg white
set -g __icon_status_fail FAIL
set -g __gpy_last_segment_bg cyan

segment_status_render last

if test "$__gpy_last_segment_bg" = magenta
    check "fail path: tracks __gpy_last_segment_bg to the configured fail background" pass
else
    check "fail path: tracks __gpy_last_segment_bg to the configured fail background (got: '$__gpy_last_segment_bg')" fail
end

# --- successful path, theme color UNSET: must track the resolved literal
# fallback ("green"), not the raw (empty) __color_status_ok_bg. Setting the
# tracker to an empty string would silently break the next segment's chevron.

set -g __gpy_last_status 0
set -e __color_status_ok_bg
set -e __color_status_ok_fg
set -g __gpy_last_segment_bg sentinel

segment_status_render last

if test "$__gpy_last_segment_bg" = green
    check "ok path (theme color unset): tracks the resolved literal fallback 'green'" pass
else
    check "ok path (theme color unset): tracks the resolved literal fallback 'green' (got: '$__gpy_last_segment_bg')" fail
end

# --- failed path, theme color UNSET: must track the resolved literal
# fallback ("red"), not the raw (empty) __color_status_fail_bg.

set -g __gpy_last_status 1
set -e __color_status_fail_bg
set -e __color_status_fail_fg
set -g __gpy_last_segment_bg sentinel

segment_status_render last

if test "$__gpy_last_segment_bg" = red
    check "fail path (theme color unset): tracks the resolved literal fallback 'red'" pass
else
    check "fail path (theme color unset): tracks the resolved literal fallback 'red' (got: '$__gpy_last_segment_bg')" fail
end

functions -e gpy_section_standalone

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count status segment tests passed"
