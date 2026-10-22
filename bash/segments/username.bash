# Starship `username` parity: shown when root or under sudo.

__gpy_segment_username_detect() {
    [[ "${__gpy_is_root:-0}" == "1" || "${__gpy_is_sudo:-0}" == "1" || "${__username_show_always:-0}" == "1" ]]
}

__gpy_segment_username() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    # The agent draws the pill when the theme has a format; no reply (agent
    # down, disabled, or an error) falls through to the local pill.
    if [[ -n "${__username_format:-}" ]] && declare -F __gpy_request_username >/dev/null 2>&1; then
        local rendered
        if rendered="$(__gpy_request_username "${USER:-}" "$is_last" "$prev_bg" "$is_first")"; then
            echo "$rendered"
            return 0
        fi
    fi

    # No format (or no reply): draw locally, with the one leading space every pill has.
    local label=" ${USER:-}"
    [[ -n "${__icon_username:-}" ]] && label=" $__icon_username ${USER:-}"

    __gpy_prompt_escape "$label" label
    __gpy_section_standalone "${__color_username_bg:-}" "${__color_username_fg:-}" \
        "$label" "$is_last" "$is_first"
}
