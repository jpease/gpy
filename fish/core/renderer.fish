# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# RENDERING UTILITIES
# ============================================================================
#
# Public helper surface for segment authors:
# - gpy_section_start
# - gpy_section_append
# - gpy_section_end
# - gpy_section_standalone
#
# Built-in segments and community plugin segments may rely on these helpers.
# Keep the function names and basic call contract stable.

# Segment delimiter configuration (can be overridden by themes)
set -q __segment_delim_first; or set -g __segment_delim_first "|"
set -q __segment_delim_start; or set -g __segment_delim_start "["
set -q __segment_delim_end; or set -g __segment_delim_end "]"
set -q __segment_delim_last; or set -g __segment_delim_last ")"

# Track current segment position for delimiter selection
set -g __gpy_segment_position first

function gpy_section_start --argument-names bg fg icon
    # Determine which delimiter to use based on position
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

    # Resolve magic color keywords for foreground
    switch $delim_fg
        case match_bg
            set delim_fg $bg
        case match_text
            set delim_fg $fg
        case transparent
            set delim_fg ""
    end

    # Resolve magic color keywords for background
    switch $delim_bg
        case match_bg
            set delim_bg $bg
        case match_text
            set delim_bg $fg
        case transparent
            set delim_bg ""
    end

    # Ensure colors have fallbacks if empty
    test -z "$bg"; and set bg normal
    test -z "$delim_fg"; and set delim_fg normal
    test -z "$delim_bg"; and set delim_bg normal

    # Draw delimiter or powerline chevron
    if test -n "$start_delim"
        set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$start_delim"
    else
        # Fallback to powerline style
        set_color -b "$bg" "$__prompt_base_bg"
        printf "%s" "$__icon_powerline_segment_start"
    end

    # Print icon/text on segment bg with segment fg
    set_color -b "$bg" "$fg"
    test -n "$icon"; and printf "%s" "$icon"

    # After first segment, all others are "middle"
    if test "$__gpy_segment_position" = first
        set -g __gpy_segment_position middle
    end
end

function gpy_section_end --argument-names bg is_last
    # Determine which delimiter to use based on position
    #
    # `true` is the first-party convention (#613: fish_prompt.fish's dispatch
    # loop and every built-in segment). `last` is accepted too, for backward
    # compatibility with third-party plugin segments written against the
    # pre-#613 dispatch, which passed the literal "last".
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

    # Resolve magic color keywords for foreground
    switch $delim_fg
        case match_bg
            set delim_fg $bg
        case match_text
            # For end delimiters, we don't have fg passed in
            # Use the last known fg or fallback to normal
            set delim_fg normal
        case transparent
            set delim_fg ""
    end

    # Resolve magic color keywords for background
    switch $delim_bg
        case match_bg
            set delim_bg $bg
        case match_text
            set delim_bg normal
        case transparent
            set delim_bg ""
    end

    # Ensure colors have fallbacks if empty
    test -z "$bg"; and set bg normal
    test -z "$delim_fg"; and set delim_fg normal
    test -z "$delim_bg"; and set delim_bg normal

    # Same-background gap space before a non-last closing cap: the segment's
    # own backdrop continues right up to the (transparent-background) glyph
    # instead of falling through to the terminal default early.
    if test "$is_last" != true -a "$is_last" != last
        set -l gap_bg $bg
        test "$gap_bg" = transparent; and set gap_bg ""
        test -z "$gap_bg"; and set gap_bg normal
        set_color -b "$gap_bg" normal
        printf " "
    end

    # Draw delimiter or powerline chevron
    if test -n "$end_delim"
        set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$end_delim"
    else
        # Fallback to powerline style
        set_color -b "$__prompt_base_bg" "$bg"
        printf "%s" "$__icon_powerline_segment_end"
    end

    # Return to neutral
    set_color normal
end

function gpy_section_append --argument-names bg fg content
    # Append content to current section without transition chevron
    # The segment's own bg/fg may be the `transparent` keyword (e.g. a flat theme's
    # shell-rendered clock). Map it to the terminal default; otherwise `set_color`
    # errors with "Unknown color 'transparent'" and prints to stderr every render.
    test "$bg" = transparent; and set bg ""
    test "$fg" = transparent; and set fg ""

    # Ensure colors have fallbacks if empty
    test -z "$bg"; and set bg normal
    test -z "$fg"; and set fg normal
    set_color -b "$bg" "$fg"
    printf "%s" "$content"
end

function gpy_section_standalone --argument-names bg fg content is_last
    # Determine which delimiters to use based on position. `is_last` accepts
    # both "true" (first-party, #613) and the legacy "last" spelling (see
    # gpy_section_end above for why).
    set -l start_delim
    set -l end_delim
    set -l start_fg
    set -l start_bg
    set -l end_fg
    set -l end_bg

    # The segment's own bg/fg may be the `transparent` keyword (e.g. a flat theme's
    # shell-rendered clock). Map it to the terminal default BEFORE the delimiter
    # colors borrow it via `match_bg`/`match_text` below; otherwise `set_color`
    # errors with "Unknown color 'transparent'" and prints to stderr every render.
    test "$bg" = transparent; and set bg ""
    test "$fg" = transparent; and set fg ""

    switch $__gpy_segment_position
        case first
            set start_delim $__segment_delim_first
            set start_fg $__prompt_open_color
            set start_bg $__prompt_open_bg
            if test "$is_last" = true -o "$is_last" = last
                set end_delim $__segment_delim_last
                set end_fg $__prompt_close_color
                set end_bg $__prompt_close_bg
            else
                set end_delim $__segment_delim_end
                set end_fg $__segment_delimiter_color
                set end_bg $__segment_delimiter_bg
            end
            set -g __gpy_segment_position middle
        case '*'
            set start_delim $__segment_delim_start
            set start_fg $__segment_delimiter_color
            set start_bg $__segment_delimiter_bg
            if test "$is_last" = true -o "$is_last" = last
                set end_delim $__segment_delim_last
                set end_fg $__prompt_close_color
                set end_bg $__prompt_close_bg
            else
                set end_delim $__segment_delim_end
                set end_fg $__segment_delimiter_color
                set end_bg $__segment_delimiter_bg
            end
    end

    # Resolve magic color keywords for start delimiter foreground
    switch $start_fg
        case match_bg
            set start_fg $bg
        case match_text
            set start_fg $fg
        case transparent
            set start_fg ""
    end

    # Resolve magic color keywords for start delimiter background
    switch $start_bg
        case match_bg
            set start_bg $bg
        case match_text
            set start_bg $fg
        case transparent
            set start_bg ""
    end

    # Resolve magic color keywords for end delimiter foreground
    switch $end_fg
        case match_bg
            set end_fg $bg
        case match_text
            set end_fg $fg
        case transparent
            set end_fg ""
    end

    # Resolve magic color keywords for end delimiter background
    switch $end_bg
        case match_bg
            set end_bg $bg
        case match_text
            set end_bg $fg
        case transparent
            set end_bg ""
    end

    # Ensure colors have fallbacks if empty
    test -z "$bg"; and set bg normal
    test -z "$fg"; and set fg normal
    test -z "$start_fg"; and set start_fg normal
    test -z "$start_bg"; and set start_bg normal
    test -z "$end_fg"; and set end_fg normal
    test -z "$end_bg"; and set end_bg normal

    # Render start delimiter with its colors
    set_color -b "$start_bg" "$start_fg"
    printf "%s" "$start_delim"

    # Render content with fg color
    set_color -b "$bg" "$fg"
    printf "%s" "$content"

    # Same-background gap space before a non-last closing cap: the segment's
    # own backdrop continues right up to the (transparent-background) glyph
    # instead of falling through to the terminal default early.
    if test "$is_last" != true -a "$is_last" != last
        set_color -b "$bg" normal
        printf " "
    end

    # Render end delimiter with its colors
    set_color -b "$end_bg" "$end_fg"
    printf "%s" "$end_delim"

    set_color normal
end
