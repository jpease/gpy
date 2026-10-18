# SPDX-License-Identifier: GPL-3.0-or-later

# strftime spec (no leading +); must match ClockResolver::time_spec, since the agent's
# response carries it as the placeholder segment_clock_render replaces.
function __gpy_clock_date_format --description 'Build the strftime spec (no leading +) for the clock segment'
    set -l hour_format "%-I"
    set -l minute_format ":%M"
    set -l second_format ""
    set -l ampm_format " %p"

    if test "$__time_format" = 24
        set ampm_format ""
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%H"
        else
            set hour_format "%-H"
        end
    else
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%I"
        else
            set hour_format "%-I"
        end
    end

    if test "$__clock_show_seconds" = 1
        set second_format ":%S"
    end

    printf '%s%s%s%s' $hour_format $minute_format $second_format $ampm_format
end

function segment_clock_detect
    return 0
end

function segment_clock_render --argument-names is_last is_first
    set -l now
    if set -q __gpy_clock_prerendered
        # Reuse fish_prompt's single combined `date` call.
        set now $__gpy_clock_prerendered
    else
        # Fish has no native time prompt token, so one `date` fork is unavoidable.
        set -l format "+"(__gpy_clock_date_format)
        set now (string trim (date "$format"))
    end

    # Agent response carries the bare spec where the time goes.
    if test -n "$__clock_format"; and functions -q __gpy_request_clock
        # One-token local: an empty-list binding would shift prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l is_first_value ""
        test "$is_first" = true; and set is_first_value true
        set -l result (__gpy_request_clock $is_last_value $is_first_value "$__gpy_last_segment_bg" | string collect)
        if test -n "$result"
            printf '%s' (string replace -- (__gpy_clock_date_format) $now $result)
            # Only track bg when a pill rendered.
            set -g __gpy_last_segment_bg $__color_clock_bg
            return
        end
    end

    # No trailing pad: gpy_section_standalone already emits the gap before a closing cap.
    set -q __prompt_time_pad; or set -g __prompt_time_pad " "
    gpy_section_standalone $__color_clock_bg $__color_clock_fg "$__prompt_time_pad$now" $is_last
    set -g __gpy_last_segment_bg $__color_clock_bg
end
