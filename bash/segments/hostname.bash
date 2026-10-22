__gpy_segment_hostname_detect() {
    [[ "${__gpy_is_ssh:-0}" == "1" || "${__hostname_show_always:-0}" == "1" ]]
}

__gpy_segment_hostname() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    # The agent draws the pill when the theme has a format; no reply (agent
    # down, disabled, or an error) falls through to the local pill.
    if [[ -n "${__hostname_format:-}" ]] && declare -F __gpy_request_hostname >/dev/null 2>&1; then
        local rendered
        if rendered="$(__gpy_request_hostname "$HOSTNAME" "$is_last" "$prev_bg" "${__gpy_is_ssh:-0}" "$is_first")"; then
            echo "$rendered"
            return 0
        fi
    fi

    # No format (or no reply): draw locally, with the one leading space every pill has.
    # No-colon `${var-.}`: an explicit empty trim_at means "no trim".
    local delim="${__hostname_trim_at-.}"
    local name="$HOSTNAME"
    [[ -n "$delim" ]] && name="${HOSTNAME%%"$delim"*}"
    local label=" $name"
    [[ "${__gpy_is_ssh:-0}" == "1" && -n "${__icon_hostname:-}" ]] && label=" $__icon_hostname $name"

    __gpy_prompt_escape "$label" label
    __gpy_section_standalone "${__color_hostname_bg:-}" "${__color_hostname_fg:-}" \
        "$label" "$is_last" "$is_first"
}
