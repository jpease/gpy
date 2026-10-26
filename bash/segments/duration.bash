__gpy_segment_duration_detect() {
    local threshold="${__duration_threshold_ms:-100}"
    [[ -n "$__gpy_cmd_duration" && $__gpy_cmd_duration -ge $threshold ]]
}

__gpy_segment_duration() {
    local threshold="${__duration_threshold_ms:-100}"
    if [[ -z "$__gpy_cmd_duration" || $__gpy_cmd_duration -lt $threshold ]]; then
        return
    fi

    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    __gpy_request_duration "$__gpy_cmd_duration" "$is_last" "$prev_bg" "$is_first"
}
