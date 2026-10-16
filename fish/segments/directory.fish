# SPDX-License-Identifier: GPL-3.0-or-later

function segment_directory_detect --description "Always show directory segment"
    return 0
end

function segment_directory_render --argument-names is_last is_first --description "Render directory segment from agent ANSI"
    # Seed missing args as "" (see fish_prompt.fish's dispatch loop).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    # Memoized on (theme, PWD, position, prev_bg) to skip the IPC round-trip.
    set -l dir_key "$__gpy_theme_name:$PWD:$is_last:$is_first:$__gpy_last_segment_bg"
    set -l dir_output
    if test -n "$__gpy_dir_cache_key"; and test "$dir_key" = "$__gpy_dir_cache_key"
        set dir_output $__gpy_dir_cache_val
    else
        set dir_output (__gpy_request directory "$PWD" $is_last $__gpy_last_segment_bg $is_first)
        # Never cache a failed render: the agent-down fallback must retry next prompt.
        if test -n "$dir_output"
            set -g __gpy_dir_cache_key $dir_key
            set -g __gpy_dir_cache_val $dir_output
        end
    end
    printf '%s' $dir_output
    if test -n "$dir_output"
        set -g __gpy_last_segment_bg $__color_directory_bg
    end
end
