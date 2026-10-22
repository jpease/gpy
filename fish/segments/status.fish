# SPDX-License-Identifier: GPL-3.0-or-later

function segment_status_detect
    return 0
end

function segment_status_render --argument-names is_last is_first
    set -q __gpy_last_status; or set -l __gpy_last_status 0
    set -l success 0
    set -l bg_color
    set -l fg_color
    set -l icon
    if test "$__gpy_last_status" -eq 0
        set success 1
        set bg_color (set -q __color_status_ok_bg && echo $__color_status_ok_bg || echo green)
        set fg_color (set -q __color_status_ok_fg && echo $__color_status_ok_fg || echo black)
        set icon "$__icon_status_ok"
    else
        set bg_color (set -q __color_status_fail_bg && echo $__color_status_fail_bg || echo red)
        set fg_color (set -q __color_status_fail_fg && echo $__color_status_fail_fg || echo black)
        set icon "$__icon_status_fail"
    end

    # The agent draws the pill when the theme has a format; an empty reply
    # (no agent, or no ipc.fish in disabled mode) falls through to the local pill.
    if test -n "$__status_format"; and functions -q __gpy_request_exit_status
        # One-token locals: an empty-list binding would shift prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l is_first_value ""
        test "$is_first" = true; and set is_first_value true
        set -l result (__gpy_request_exit_status $success $is_last_value $is_first_value "$__gpy_last_segment_bg" | string collect)
        if test -n "$result"
            printf '%s' "$result"
            # Use the resolved $bg_color: the raw theme var may be unset.
            set -g __gpy_last_segment_bg $bg_color
            return
        end
    end

    # No format (or no reply): draw locally, with the one leading space every pill has.
    gpy_section_standalone $bg_color $fg_color " $icon" $is_last
    set -g __gpy_last_segment_bg $bg_color
end
