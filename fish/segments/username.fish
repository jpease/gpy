# SPDX-License-Identifier: GPL-3.0-or-later
# Starship `username` parity: shown when root, under sudo, or over SSH. Opt-in.
# Root (euid 0) draws the theme's root colours; everyone else the user colours.

function __gpy_username_should_show --argument-names is_root is_sudo is_ssh show_always
    test "$show_always" -eq 1; or test "$is_root" -eq 1; or test "$is_sudo" -eq 1; or test "$is_ssh" -eq 1
end

function segment_username_detect
    __gpy_username_should_show $__gpy_is_root $__gpy_is_sudo $__gpy_is_ssh $__username_show_always
end

function segment_username_render --argument-names is_last is_first
    # The agent draws the pill when the theme has a format; no reply (agent
    # down, disabled, or an error) falls through to the local pill.
    set -l bg $__color_username_user_bg
    set -l fg $__color_username_user_fg
    if test "$__gpy_is_root" = 1
        set bg $__color_username_bg
        set fg $__color_username_fg
    end

    if test -n "$__username_format"; and functions -q __gpy_request_username
        # Format content is presence-only; the agent re-derives it.
        # One-token locals: an empty-list binding would shift prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l is_first_value ""
        test "$is_first" = true; and set is_first_value true
        set -l is_root_value 0
        test "$__gpy_is_root" = 1; and set is_root_value 1
        set -l result (__gpy_request_username "$USER" $is_last_value "$__gpy_last_segment_bg" $is_root_value $is_first_value | string collect)
        if test -n "$result"
            printf '%s' "$result"
            set -g __gpy_last_segment_bg $bg
            return
        end
    end

    # No format (or no reply): draw locally, with the one leading space every pill has.
    set -l label " $USER"
    test -n "$__icon_username"; and set label " $__icon_username $USER"
    gpy_section_standalone $bg $fg "$label" $is_last
    set -g __gpy_last_segment_bg $bg
end
