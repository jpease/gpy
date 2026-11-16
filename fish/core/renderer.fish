# SPDX-License-Identifier: GPL-3.0-or-later
# Public API for built-in and plugin segments: keep names and call contract stable.

set -q __segment_delim_first; or set -g __segment_delim_first "|"
set -q __segment_delim_start; or set -g __segment_delim_start "["
set -q __segment_delim_end; or set -g __segment_delim_end "]"
set -q __segment_delim_last; or set -g __segment_delim_last ")"

set -g __gpy_segment_position first

# Colour depth of the pills Fish draws itself, the agent's `ColorDepth`
# (gpy-agent/src/formatter/style_encoder.rs): GPY_COLOR_DEPTH (`truecolor`
# or `24bit`, `256`, `16`, `none`; case-insensitive, surrounding blanks
# ignored, any other value ignored), else 256 on Apple Terminal
# (TERM_PROGRAM) and TERM=linux, else 24-bit. Read once, at load, as the
# agent does. Empty is that default 24-bit depth, which set_color draws
# exactly as before; every other depth writes its own SGR (__gpy_set_color).
function __gpy_color_depth_detect --description 'Set __gpy_color_depth from GPY_COLOR_DEPTH and the terminal'
    set -l depth (string trim -- "$GPY_COLOR_DEPTH" | string lower)
    switch "$depth"
        case truecolor 24bit
            set -g __gpy_color_depth truecolor
        case 256 16 none
            set -g __gpy_color_depth $depth
        case '*'
            if test "$TERM_PROGRAM" = Apple_Terminal; or test "$TERM" = linux
                set -g __gpy_color_depth 256
            else
                set -g __gpy_color_depth ""
            end
    end
end
__gpy_color_depth_detect

# `set_color [-b BG] FG` at __gpy_color_depth.
function __gpy_set_color --description 'set_color at the colour depth'
    if test -z "$__gpy_color_depth"
        set_color $argv
        return
    end
    set -l bg
    if test "$argv[1]" = -b
        set bg $argv[2]
        set -e argv[1..2]
    end
    set -l params (__gpy_sgr_param fg "$argv[1]")
    set -q bg[1]; and set -a params (__gpy_sgr_param bg "$bg")
    set -q params[1]; and printf '\e[%sm' (string join ';' -- $params)
    return 0
end

# SGR parameter of COLOR as a foreground or background (KIND fg|bg) at a
# non-default __gpy_color_depth, as the agent and Zsh's __gpy_sgr_param
# write it: names 30-37 and 90-97 (40-47, 100-107), `#rrggbb` (or set_color's
# `rrggbb`, `#rgb`, `rgb`) at the depth, anything else (normal, default,
# empty) the terminal default 39 (49). Prints nothing at depth none.
function __gpy_sgr_param --argument-names kind color
    test "$__gpy_color_depth" = none; and return 0
    set -l base 30
    test "$kind" = bg; and set base 40
    set -l named black red green yellow blue magenta cyan white
    set -l index (contains -i -- "$color" $named)
    if test -n "$index"
        math $base + $index - 1
        return 0
    end
    switch "$color"
        case purple
            math $base + 5
            return 0
        case brblack bright_black bright-black gray grey
            math $base + 60
            return 0
        case brred bright_red bright-red
            math $base + 61
            return 0
        case brgreen bright_green bright-green
            math $base + 62
            return 0
        case bryellow bright_yellow bright-yellow
            math $base + 63
            return 0
        case brblue bright_blue bright-blue
            math $base + 64
            return 0
        case brmagenta bright_magenta bright-purple
            math $base + 65
            return 0
        case brcyan bright_cyan bright-cyan
            math $base + 66
            return 0
        case brwhite bright_white bright-white
            math $base + 67
            return 0
    end
    set -l hex (string match -r -- '\A#?([0-9a-fA-F]{6}|[0-9a-fA-F]{3})\z' "$color")[2]
    if test -z "$hex"
        math $base + 9
        return 0
    end
    test (string length -- $hex) -eq 3; and set hex (string replace -ra '(.)' '$1$1' -- $hex)
    set -l r (math 0x(string sub -s 1 -l 2 -- $hex))
    set -l g (math 0x(string sub -s 3 -l 2 -- $hex))
    set -l b (math 0x(string sub -s 5 -l 2 -- $hex))
    switch $__gpy_color_depth
        case 256
            echo (math $base + 8)";5;"(__gpy_rgb_to_256 $r $g $b)
        case 16
            set -l code (__gpy_rgb_to_16 $r $g $b)
            test "$kind" = bg; and set code (math $code + 10)
            echo $code
        case '*'
            echo (math $base + 8)";2;$r;$g;$b"
    end
end

# The xterm 256-colour index nearest to R G B: the grey ramp for greys, else
# the 6x6x6 cube.
function __gpy_rgb_to_256 --argument-names r g b
    if test $r -eq $g -a $g -eq $b
        if test $r -le 7
            echo 16
        else if test $r -ge 248
            echo 231
        else
            set -l step (math "floor(($r - 3) / 10)")
            test $step -gt 23; and set step 23
            math 232 + $step
        end
        return 0
    end
    set -l levels
    for c in $r $g $b
        if test $c -lt 48
            set -a levels 0
        else if test $c -lt 115
            set -a levels 1
        else if test $c -lt 155
            set -a levels 2
        else if test $c -lt 195
            set -a levels 3
        else if test $c -lt 235
            set -a levels 4
        else
            set -a levels 5
        end
    end
    math "16 + 36 * $levels[1] + 6 * $levels[2] + $levels[3]"
end

# The 16 ANSI colours as xterm draws them, in SGR order (30-37, then 90-97).
set -g __gpy_ansi16_rgb 0,0,0 205,0,0 0,205,0 205,205,0 0,0,238 205,0,205 0,205,205 229,229,229 \
    127,127,127 255,0,0 0,255,0 255,255,0 92,92,255 255,0,255 0,255,255 255,255,255

# Foreground code (30-37, 90-97) of the ANSI colour nearest to R G B; the
# first of equally near ones.
function __gpy_rgb_to_16 --argument-names r g b
    set -l best 0
    set -l best_distance -1
    set -l i 0
    for rgb in $__gpy_ansi16_rgb
        set -l p (string split , -- $rgb)
        set -l d (math "($r - $p[1])^2 + ($g - $p[2])^2 + ($b - $p[3])^2")
        if test $best_distance -lt 0 -o $d -lt $best_distance
            set best_distance $d
            set best $i
        end
        set i (math $i + 1)
    end
    if test $best -lt 8
        math 30 + $best
    else
        math 82 + $best
    end
end

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
        __gpy_set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$start_delim"
    else
        __gpy_set_color -b "$bg" "$__prompt_base_bg"
        printf "%s" "$__icon_powerline_segment_start"
    end

    __gpy_set_color -b "$bg" "$fg"
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
        __gpy_set_color -b "$gap_bg" normal
        printf " "
    end

    if test -n "$end_delim"
        __gpy_set_color -b "$delim_bg" "$delim_fg"
        printf "%s" "$end_delim"
    else
        __gpy_set_color -b "$__prompt_base_bg" "$bg"
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
    __gpy_set_color -b "$bg" "$fg"
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

    __gpy_set_color -b "$start_bg" "$start_fg"
    printf "%s" "$start_delim"

    __gpy_set_color -b "$bg" "$fg"
    printf "%s" "$content"

    if test "$is_last" != true -a "$is_last" != last
        __gpy_set_color -b "$bg" normal
        printf " "
    end

    __gpy_set_color -b "$end_bg" "$end_fg"
    printf "%s" "$end_delim"

    set_color normal
end
