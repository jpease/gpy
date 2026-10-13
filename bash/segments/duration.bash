# bash/segments/duration.bash

__gpy_segment_duration_detect() {
    local threshold="${__duration_threshold_ms:-2000}"
    [[ -n "$__gpy_cmd_duration" && $__gpy_cmd_duration -ge $threshold ]]
}

__gpy_segment_duration() {
    # Only show if duration exceeds threshold
    local threshold="${__duration_threshold_ms:-2000}"
    if [[ -z "$__gpy_cmd_duration" || $__gpy_cmd_duration -lt $threshold ]]; then
        return
    fi

    # Duration is always agent-rendered: the agent applies the theme template
    # and returns pre-formatted ANSI. is_last/is_first arrive as "true"/""
    # (init.bash's dispatch-loop convention).
    local is_last="${1:-}"
    # prev_bg: threaded by the render loop for the opening powerline chevron.
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    __gpy_request_duration "$__gpy_cmd_duration" "$is_last" "$prev_bg" "$is_first"
}
