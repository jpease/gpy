# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# EXIT STATUS SEGMENT
# ============================================================================

function segment_status_detect
    return 0 # Always show status
end

function segment_status_render --argument-names is_last
    set -q __gpy_last_status; or set -l __gpy_last_status 0
    if test "$__gpy_last_status" -eq 0
        # Use colors from theme (fallback to green/black if not set)
        set -l bg_color (set -q __color_status_ok_bg && echo $__color_status_ok_bg || echo green)
        set -l fg_color (set -q __color_status_ok_fg && echo $__color_status_ok_fg || echo black)
        gpy_section_standalone $bg_color $fg_color "$__icon_status_ok" $is_last
        # Track this segment's bg so the next segment can render a powerline
        # chevron. Status always renders a pill (segment_status_detect never
        # hides it), so this is unconditional like clock.fish — but it must be
        # the resolved $bg_color local, not the raw (possibly unset) theme
        # var, or an unconfigured theme silently blanks the tracker.
        set -g __gpy_last_segment_bg $bg_color
    else
        set -l bg_color (set -q __color_status_fail_bg && echo $__color_status_fail_bg || echo red)
        set -l fg_color (set -q __color_status_fail_fg && echo $__color_status_fail_fg || echo black)
        gpy_section_standalone $bg_color $fg_color "$__icon_status_fail" $is_last
        # Track this segment's bg (see comment in the ok branch above).
        set -g __gpy_last_segment_bg $bg_color
    end
end
