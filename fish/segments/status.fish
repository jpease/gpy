# SPDX-License-Identifier: GPL-3.0-or-later

function segment_status_detect
    return 0
end

function segment_status_render --argument-names is_last
    set -q __gpy_last_status; or set -l __gpy_last_status 0
    if test "$__gpy_last_status" -eq 0
        set -l bg_color (set -q __color_status_ok_bg && echo $__color_status_ok_bg || echo green)
        set -l fg_color (set -q __color_status_ok_fg && echo $__color_status_ok_fg || echo black)
        gpy_section_standalone $bg_color $fg_color "$__icon_status_ok" $is_last
        # Use the resolved $bg_color: the raw theme var may be unset.
        set -g __gpy_last_segment_bg $bg_color
    else
        set -l bg_color (set -q __color_status_fail_bg && echo $__color_status_fail_bg || echo red)
        set -l fg_color (set -q __color_status_fail_fg && echo $__color_status_fail_fg || echo black)
        gpy_section_standalone $bg_color $fg_color "$__icon_status_fail" $is_last
        set -g __gpy_last_segment_bg $bg_color
    end
end
