# bash/segments/directory.bash

__gpy_segment_directory() {
    # Directory is always agent-rendered (#199): the agent applies the theme
    # template and returns pre-formatted ANSI. The shell no longer renders it
    # locally and no longer consumes `__color_directory_*`.
    # is_last/is_first arrive already as "true"/"" (#613: init.bash's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    local is_last="${1:-}"
    # prev_bg: threaded by the render loop for the opening powerline chevron.
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    __gpy_request "directory" "$PWD" "bash-prompt" "$is_last" "$prev_bg" "$is_first"
}
