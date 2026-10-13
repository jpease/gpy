# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# DIRECTORY SEGMENT
# ============================================================================

function segment_directory_detect --description "Always show directory segment"
    return 0 # Always show directory
end

function segment_directory_render --argument-names is_last is_first --description "Render directory segment from agent ANSI"
    # Seed missing args as "" (see fish_prompt.fish's dispatch loop).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    # Directory is always agent-rendered: the agent applies the theme
    # template and returns pre-formatted ANSI.
    # Memoized (#343): skip the IPC round-trip + fork entirely when the input
    # tuple (theme identity + PWD + is_last + is_first + prev_bg) matches the last render.
    set -l dir_key "$__gpy_theme_name:$PWD:$is_last:$is_first:$__gpy_last_segment_bg"
    set -l dir_output
    if test -n "$__gpy_dir_cache_key"; and test "$dir_key" = "$__gpy_dir_cache_key"
        set dir_output $__gpy_dir_cache_val
    else
        set dir_output (__gpy_request directory "$PWD" $is_last $__gpy_last_segment_bg $is_first)
        # Never cache an empty/failed render: the agent-down fallback must
        # retry on the very next prompt, not get stuck forever.
        if test -n "$dir_output"
            set -g __gpy_dir_cache_key $dir_key
            set -g __gpy_dir_cache_val $dir_output
        end
    end
    printf '%s' $dir_output
    # Track this segment's bg so the next segment can render a powerline chevron.
    if test -n "$dir_output"
        set -g __gpy_last_segment_bg $__color_directory_bg
    end
end
