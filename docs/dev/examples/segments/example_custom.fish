# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# EXAMPLE CUSTOM SEGMENT - Battery Status (macOS)
# ============================================================================

# Optional: Initialize segment-specific state.
# This function is called once when the prompt is first loaded.
# It's useful for caching expensive checks.
# function segment_battery_init
#     if not __has_binary pmset
#         set -g __battery_is_supported 0
#     else
#         set -g __battery_is_supported 1
#     end
# end

function segment_battery_detect
    # Only show on macOS laptops
    test (uname) = Darwin; and __has_binary pmset
end

function segment_battery_render --argument-names is_last
    set -l battery_info (pmset -g batt | grep -E "([0-9]+%)")
    if test -n "$battery_info"
        set -l percentage (echo $battery_info | grep -o '[0-9]*%' | head -1)
        set -l number (echo $percentage | string replace '%' '')

        set -l icon "🔋"
        set -l color green

        if test $number -le 20
            set icon "🪫"
            set color red
        else if test $number -le 50
            set icon "🔋"
            set color yellow
        end

        gpy_section_start $color black "$icon $percentage"
        gpy_section_end $color
    end
end
