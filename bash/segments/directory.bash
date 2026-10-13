# bash/segments/directory.bash

__gpy_segment_directory() {
    # Directory is always agent-rendered: the agent applies the theme template
    # and returns pre-formatted ANSI. is_last/is_first arrive as "true"/""
    # (init.bash's dispatch-loop convention).
    local is_last="${1:-}"
    # prev_bg: threaded by the render loop for the opening powerline chevron.
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    __gpy_request "directory" "$PWD" "bash-prompt" "$is_last" "$prev_bg" "$is_first"
}
