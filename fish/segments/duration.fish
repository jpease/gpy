# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# COMMAND DURATION SEGMENT
# ============================================================================

function segment_duration_detect
    # Use threshold from theme (default 100ms if not set)
    set -l threshold (set -q __duration_threshold_ms && echo $__duration_threshold_ms || echo 100)
    test -n "$CMD_DURATION" && test "$CMD_DURATION" -ge $threshold
end

function segment_duration_render --argument-names is_last is_first
    # A caller that passes fewer than 2 args leaves the corresponding
    # --argument-names binding an EMPTY LIST, not an empty string -- normalize
    # so every unquoted use below always expands to exactly one word instead
    # of silently collapsing and shifting a later positional argument (#629-class).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    # Duration is always agent-rendered (#199): the agent applies the theme
    # template and returns pre-formatted ANSI. The shell no longer renders it
    # locally and no longer consumes `__color_duration_*`. The detect gate above
    # (using `__duration_threshold_ms`) still decides visibility.
    set -l ms $CMD_DURATION
    test -z "$ms"; and set ms 0
    # is_last/is_first arrive already as "true"/"" (#613: fish_prompt.fish's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    set -l dur_output (__gpy_request_duration "$ms" $is_last $__gpy_last_segment_bg $is_first)
    printf '%s' $dur_output
    # Track this segment's bg so the next segment can render a powerline chevron.
    if test -n "$dur_output"
        set -g __gpy_last_segment_bg $__color_duration_bg
    end
end
