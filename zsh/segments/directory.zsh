function __gpy_segment_directory() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    __gpy_request "directory" "$PWD" "zsh-prompt" "$is_last" "$prev_bg" "$is_first"
}
