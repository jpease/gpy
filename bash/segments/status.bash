__gpy_segment_status() {
    local is_last="${1:-}"
    local is_first="${3:-}"

    local bg fg icon
    if [[ "${__gpy_last_status:-0}" -eq 0 ]]; then
        bg="${__color_status_ok_bg:-green}"
        fg="${__color_status_ok_fg:-black}"
        __gpy_status_icon ok icon
    else
        bg="${__color_status_fail_bg:-red}"
        fg="${__color_status_fail_fg:-black}"
        __gpy_status_icon fail icon
    fi

    __gpy_prompt_escape "$icon" icon
    __gpy_section_standalone "$bg" "$fg" "$icon" "$is_last" "$is_first"
}
