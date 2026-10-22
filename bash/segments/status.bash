__gpy_segment_status() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    local bg fg icon success=0
    if [[ "${__gpy_last_status:-0}" -eq 0 ]]; then
        success=1
        bg="${__color_status_ok_bg:-green}"
        fg="${__color_status_ok_fg:-black}"
        __gpy_status_icon ok icon
    else
        bg="${__color_status_fail_bg:-red}"
        fg="${__color_status_fail_fg:-black}"
        __gpy_status_icon fail icon
    fi

    # The agent draws the pill when the theme has a format; an empty reply
    # (no agent) falls through to the local pill.
    if [[ -n "${__status_format:-}" ]] && declare -F __gpy_request_exit_status >/dev/null 2>&1; then
        local rendered
        rendered="$(__gpy_request_exit_status "$success" "$is_last" "$prev_bg" "$is_first")"
        if [[ -n "$rendered" ]]; then
            echo "$rendered"
            return 0
        fi
    fi

    # No format (or no reply): draw locally, with the one leading space every pill has.
    icon=" $icon"
    __gpy_prompt_escape "$icon" icon
    __gpy_section_standalone "$bg" "$fg" "$icon" "$is_last" "$is_first"
}
