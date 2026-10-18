# SPDX-License-Identifier: GPL-3.0-or-later
# Starship `username` parity: shown when root or under sudo. Opt-in.

function __gpy_username_should_show --argument-names is_root is_sudo show_always
    test "$show_always" -eq 1; or test "$is_root" -eq 1; or test "$is_sudo" -eq 1
end

function segment_username_detect
    __gpy_username_should_show $__gpy_is_root $__gpy_is_sudo $__username_show_always
end

function segment_username_render --argument-names is_last
    if test -n "$__username_format"
        # Format content is presence-only; the agent re-derives it.
        # One-token local: an empty-list binding would shift prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l result (__gpy_request_username $USER $is_last_value $__gpy_last_segment_bg | string collect)
        if test -n "$result"
            printf '%s' "$result"
            # Only track bg when a pill rendered.
            set -g __gpy_last_segment_bg $__color_username_bg
        end
        return
    end

    set -l label "$USER"
    test -n "$__icon_username"; and set label "$__icon_username $USER"
    gpy_section_standalone $__color_username_bg $__color_username_fg "$label" $is_last
    set -g __gpy_last_segment_bg $__color_username_bg
end
