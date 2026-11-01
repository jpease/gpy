# SPDX-License-Identifier: GPL-3.0-or-later
# Public API for built-in and plugin segments: keep names and call contract stable.

set -q __segment_delim_first; or set -g __segment_delim_first "|"
set -q __segment_delim_start; or set -g __segment_delim_start "["
set -q __segment_delim_end; or set -g __segment_delim_end "]"
set -q __segment_delim_last; or set -g __segment_delim_last ")"

set -g __gpy_segment_position first

function gpy_section_start --argument-names bg fg icon
    set -l start_delim
    set -l delim_fg
    set -l delim_bg
    switch $__gpy_segment_position
        case first
            set start_delim $__segment_delim_first
            set delim_fg $__prompt_open_color
            set delim_bg $__prompt_open_bg
        case '*'
            set start_delim $__segment_delim_start
            set delim_fg $__segment_delimiter_color
            set delim_bg $__segment_delimiter_bg
    end

    switch $delim_fg
        case match_bg
            set delim_fg $bg
        case match_text
            set delim_fg $fg
        case transparent
            set delim_fg ""
    end

    switch $delim_bg
        case match_bg
            set delim_bg $bg
        case match_text
            set delim_bg $fg
        case transparent
            set delim_bg ""
    end

    test -z "$bg"; and set bg normal
    test -z "$delim_fg"; and set delim_fg normal
    test -z "$delim_bg"; and set delim_bg normal

    if test -n "$start_delim"
        set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$start_delim"
    else
        set_color -b "$bg" "$__prompt_base_bg"
        printf "%s" "$__icon_powerline_segment_start"
    end

    set_color -b "$bg" "$fg"
    test -n "$icon"; and printf "%s" "$icon"

    if test "$__gpy_segment_position" = first
        set -g __gpy_segment_position middle
    end
end

function gpy_section_end --argument-names bg is_last
    set -l end_delim
    set -l delim_fg
    set -l delim_bg
    if test "$is_last" = true -o "$is_last" = last
        set end_delim $__segment_delim_last
        set delim_fg $__prompt_close_color
        set delim_bg $__prompt_close_bg
    else
        set end_delim $__segment_delim_end
        set delim_fg $__segment_delimiter_color
        set delim_bg $__segment_delimiter_bg
    end

    switch $delim_fg
        case match_bg
            set delim_fg $bg
        case match_text
            set delim_fg normal
        case transparent
            set delim_fg ""
    end

    switch $delim_bg
        case match_bg
            set delim_bg $bg
        case match_text
            set delim_bg normal
        case transparent
            set delim_bg ""
    end

    test -z "$bg"; and set bg normal
    test -z "$delim_fg"; and set delim_fg normal
    test -z "$delim_bg"; and set delim_bg normal

    # Gap space on the segment bg before a non-last closing cap.
    if test "$is_last" != true -a "$is_last" != last
        set -l gap_bg $bg
        test "$gap_bg" = transparent; and set gap_bg ""
        test -z "$gap_bg"; and set gap_bg normal
        set_color -b "$gap_bg" normal
        printf " "
    end

    if test -n "$end_delim"
        set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$end_delim"
    else
        set_color -b "$__prompt_base_bg" "$bg"
        printf "%s" "$__icon_powerline_segment_end"
    end

    set_color normal
end

function gpy_section_append --argument-names bg fg content
    # Map `transparent` to default: set_color rejects it.
    test "$bg" = transparent; and set bg ""
    test "$fg" = transparent; and set fg ""

    test -z "$bg"; and set bg normal
    test -z "$fg"; and set fg normal
    set_color -b "$bg" "$fg"
    printf "%s" "$content"
end

function gpy_section_standalone --argument-names bg fg content is_last
    __gpy_section_pill "$bg" "$fg" "$content" "$is_last"
end

# gpy_section_standalone with optional per-segment caps. caps_prefix names a
# script segment's export variables (`__gpy_segment_<tok>`, a theme's
# `[segments.<id>.open]`/`.close]`): when `<prefix>_open_icon` is set, that
# icon replaces the position's start delimiter, and a non-empty
# `<prefix>_open_icon_color`/`_open_bg_color` replaces its colours; the
# `_close_*` variables do the same for the end delimiter. A caps_prefix (plugin
# pills) also draws a last segment's theme close cap on the terminal default
# background.
function __gpy_section_pill --argument-names bg fg content is_last caps_prefix
    set -l start_delim
    set -l end_delim
    set -l start_fg
    set -l start_bg
    set -l end_fg
    set -l end_bg

    set -l default_close_bg 0
    set -l has_open 0
    set -l open_icon ""
    set -l open_fg ""
    set -l open_bg ""
    set -l has_close 0
    set -l close_icon ""
    set -l close_fg ""
    set -l close_bg ""
    if test -n "$caps_prefix"
        set default_close_bg 1
        set -l v {$caps_prefix}_open_icon
        if set -q $v
            set has_open 1
            set open_icon "$$v"
            set v {$caps_prefix}_open_icon_color
            set open_fg "$$v"
            set v {$caps_prefix}_open_bg_color
            set open_bg "$$v"
        end
        set v {$caps_prefix}_close_icon
        if set -q $v
            set has_close 1
            set close_icon "$$v"
            set v {$caps_prefix}_close_icon_color
            set close_fg "$$v"
            set v {$caps_prefix}_close_bg_color
            set close_bg "$$v"
        end
    end

    # Map `transparent` before match_bg/match_text borrow it: set_color rejects it.
    test "$bg" = transparent; and set bg ""
    test "$fg" = transparent; and set fg ""

    switch $__gpy_segment_position
        case first
            set start_delim $__segment_delim_first
            set start_fg $__prompt_open_color
            set start_bg $__prompt_open_bg
            set -g __gpy_segment_position middle
        case '*'
            set start_delim $__segment_delim_start
            set start_fg $__segment_delimiter_color
            set start_bg $__segment_delimiter_bg
    end
    if test "$is_last" = true -o "$is_last" = last
        set end_delim $__segment_delim_last
        set end_fg $__prompt_close_color
        if test "$default_close_bg" = 1
            set end_bg ""
        else
            set end_bg $__prompt_close_bg
        end
    else
        set end_delim $__segment_delim_end
        set end_fg $__segment_delimiter_color
        set end_bg $__segment_delimiter_bg
    end

    if test "$has_open" = 1
        set start_delim $open_icon
        test -n "$open_fg"; and set start_fg $open_fg
        test -n "$open_bg"; and set start_bg $open_bg
    end
    if test "$has_close" = 1
        set end_delim $close_icon
        test -n "$close_fg"; and set end_fg $close_fg
        test -n "$close_bg"; and set end_bg $close_bg
    end

    switch $start_fg
        case match_bg
            set start_fg $bg
        case match_text
            set start_fg $fg
        case transparent
            set start_fg ""
    end

    switch $start_bg
        case match_bg
            set start_bg $bg
        case match_text
            set start_bg $fg
        case transparent
            set start_bg ""
    end

    switch $end_fg
        case match_bg
            set end_fg $bg
        case match_text
            set end_fg $fg
        case transparent
            set end_fg ""
    end

    switch $end_bg
        case match_bg
            set end_bg $bg
        case match_text
            set end_bg $fg
        case transparent
            set end_bg ""
    end

    test -z "$bg"; and set bg normal
    test -z "$fg"; and set fg normal
    test -z "$start_fg"; and set start_fg normal
    test -z "$start_bg"; and set start_bg normal
    test -z "$end_fg"; and set end_fg normal
    test -z "$end_bg"; and set end_bg normal

    set_color -b "$start_bg" "$start_fg"
    printf "%s" "$start_delim"

    set_color -b "$bg" "$fg"
    printf "%s" "$content"

    if test "$is_last" != true -a "$is_last" != last
        set_color -b "$bg" normal
        printf " "
    end

    set_color -b "$end_bg" "$end_fg"
    printf "%s" "$end_delim"

    set_color normal
end
