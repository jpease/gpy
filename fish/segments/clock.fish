# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# CLOCK SEGMENT
# ============================================================================

# Build the strftime(3) spec (without the leading "+") for the clock
# segment's active configuration (12h/24h, leading zero, seconds). It is the
# same spec, character for character, as the agent's
# `ClockResolver::time_spec` and the Bash/Zsh copies: when the agent renders
# the clock (#844), its response carries this spec where the time goes and
# segment_clock_render swaps in the formatted time, so the two must agree.
# Shared with fish_prompt (#342), which folds this format into the
# once-per-render epoch `date` call when the clock segment is enabled, instead
# of forking `date` again here for a second time.
function __gpy_clock_date_format --description 'Build the strftime spec (no leading +) for the clock segment'
    # Default: 12-hour format without leading zero (%-I = 1-12, %I = 01-12)
    # 24-hour: %H = 00-23, %-H = 0-23

    set -l hour_format "%-I" # Default: 12-hour without leading zero
    set -l minute_format ":%M" # Always show minutes
    set -l second_format "" # Default: no seconds
    set -l ampm_format " %p" # Default: show AM/PM for 12-hour

    # Check if 24-hour format
    if test "$__time_format" = 24
        set ampm_format "" # No AM/PM in 24-hour format
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%H" # 00-23
        else
            set hour_format "%-H" # 0-23
        end
    else
        # 12-hour format
        if test "$__clock_show_leading_zero" = 1
            set hour_format "%I" # 01-12
        else
            set hour_format "%-I" # 1-12
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

function segment_clock_render --argument-names is_last is_first
    set -l now
    if set -q __gpy_clock_prerendered
        # fish_prompt already forked a single combined `date` call for the epoch
        # and the clock text this render (#342); reuse it instead of forking again.
        set now $__gpy_clock_prerendered
    else
        # Fallback (e.g. clock enabled after fish_prompt's check, or the
        # combined call produced no clock field): build and format directly.
        # `date` here is the accepted exception for Fish's clock segment (#167): bash
        # uses `\D{...}` and zsh uses `%D{...}` natively, but Fish has no built-in
        # formatted time token, so one `date` fork per render is unavoidable when the
        # clock is enabled. An agent-side time-broadcast approach was considered and
        # deferred as disproportionate infrastructure for a single per-prompt fork.
        set -l format "+"(__gpy_clock_date_format)
        set now (string trim (date "$format"))
    end

    # Agent-rendered whenever the theme sets `[segments.clock].format` (the
    # exported `__clock_format` presence flag, as for hostname/username), the
    # same rule as Bash and Zsh (#844). The agent owns the powerline caps, the
    # delimiter colors and the template; its response carries the bare strftime
    # spec where the time goes, which is replaced with the time formatted above.
    # is_last/is_first arrive already as "true"/"" (#613) but are normalized into
    # always-one-token locals: a caller that passes zero arguments leaves the
    # --argument-names binding an EMPTY LIST, which would collapse on unquoted
    # expansion and shift $__gpy_last_segment_bg into the wrong slot.
    if test -n "$__clock_format"; and functions -q __gpy_request_clock
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l is_first_value ""
        test "$is_first" = true; and set is_first_value true
        set -l result (__gpy_request_clock $is_last_value $is_first_value "$__gpy_last_segment_bg")
        if test -n "$result"
            printf '%s' (string replace -- (__gpy_clock_date_format) $now $result)
            # Track this segment's bg for the next segment's powerline chevron,
            # but only when a pill actually rendered: an empty/failed response
            # emits nothing, so the tracker keeps pointing at whatever segment
            # last actually rendered.
            set -g __gpy_last_segment_bg $__color_clock_bg
            return
        end
    end

    # Local path: no clock template in the theme, or the agent is unreachable.
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
