# bash/segments/status.bash
#
# Exit-status pill, drawn by the shared shell-side renderer so it matches
# fish's segment_status_render for the same theme (#844): a themed pill
# (`[segments.status]` ok/fail icon, text and background colors), capped like
# every other segment. The render loop reads the previous command's exit
# status from `__gpy_last_status`, and takes the pill's background for the
# next segment's chevron from `__gpy_segment_bg status`, which picks the ok or
# fail background by that same variable.
#
# is_last/is_first arrive already as "true"/"" (#613: init.bash's
# dispatch-loop convention).

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
