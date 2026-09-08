# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# CLOCK SEGMENT
# ============================================================================

# Build the `date`(1) format spec (without the leading "+") for the clock
# segment's active configuration (12h/24h, leading zero, seconds). Shared with
# fish_prompt (#342), which folds this format into the once-per-render epoch
# `date` call when the clock segment is enabled, instead of forking `date`
# again here for a second time.
function __gpy_clock_date_format --description 'Build the date(1) format spec (no leading +) for the clock segment'
    # Default: 12-hour format without leading zero (%l = 1-12, %I = 01-12)
    # 24-hour: %H = 00-23, %k = 0-23

    set -l hour_format "%l" # Default: 12-hour without leading zero
    set -l minute_format ":%M" # Always show minutes
    set -l second_format "" # Default: no seconds
    set -l ampm_format " %p" # Default: show AM/PM for 12-hour

    # Check if 24-hour format
    if test "$__time_format" = 24
        set ampm_format "" # No AM/PM in 24-hour format
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%H" # 00-23
        else
            set hour_format "%k" # 0-23
        end
    else
        # 12-hour format
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%I" # 01-12
        else
            set hour_format "%l" # 1-12
        end
    end

    # Check if showing seconds
    if test "$__clock_show_seconds" = 1
        set second_format ":%S"
    end

    printf '%s%s%s%s' $hour_format $minute_format $second_format $ampm_format
end

function segment_clock_detect
    return 0 # Always show clock
end

function segment_clock_render --argument-names is_last
    set -l now
    if set -q __gpy_clock_prerendered
        # fish_prompt already forked a single combined `date` call for the epoch
        # and the clock text this render (#342); reuse it instead of forking again.
        set now $__gpy_clock_prerendered
    else
        # Fallback (e.g. clock enabled after fish_prompt's check, or the
        # combined call produced no clock field): build and format directly.
        # `date` here is the accepted exception for Fish's clock segment (#167): bash
        # uses `\t` and zsh uses `%D{…}` natively, but Fish has no built-in formatted
        # time token, so one `date` fork per render is unavoidable when the clock is
        # enabled. An agent-side time-broadcast approach was considered and deferred as
        # disproportionate infrastructure for a single per-prompt fork.
        set -l format "+"(__gpy_clock_date_format)
        set now (string trim (date "$format"))
    end
    # Leading pad so the time doesn't sit flush against the segment's opening
    # cap, mirroring the other segments' `[ $content]` leading space. No
    # trailing pad: gpy_section_standalone/gpy_section_end already emit a
    # same-background gap space before a non-last closing cap, so adding one
    # here too would double it up. Default to a single space when no theme
    # overrides it.
    set -q __prompt_time_pad; or set -g __prompt_time_pad " "
    gpy_section_standalone $__color_clock_bg $__color_clock_fg "$__prompt_time_pad$now" $is_last
    # Track this segment's bg so the next segment can render a powerline chevron.
    set -g __gpy_last_segment_bg $__color_clock_bg
end
