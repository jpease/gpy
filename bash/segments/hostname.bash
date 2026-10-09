# bash/segments/hostname.bash

__gpy_segment_hostname_detect() {
    [[ "${__gpy_is_ssh:-0}" == "1" || "${__hostname_show_always:-0}" == "1" ]]
}

__gpy_segment_hostname() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.bash's dispatch-loop
    # convention, matching __gpy_request_hostname's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__hostname_format:-}" ]]; then
        __gpy_request_hostname "$HOSTNAME" "$is_last" "$prev_bg" "${__gpy_is_ssh:-0}"
        return
    fi

    # Local path, drawn by the shared shell-side renderer so it matches fish's
    # pure-fish path for the same theme (#844). The icon is Starship's
    # ssh_symbol: drawn only in SSH sessions (#826).
    # NOTE: `${var-default}` (no colon) so an *unset* var defaults to "." but an
    # explicit empty string (the "no trim" contract) is preserved rather than
    # being collapsed to the default by the colon-form `${var:-default}`.
    local delim="${__hostname_trim_at-.}"
    local name="$HOSTNAME"
    [[ -n "$delim" ]] && name="${HOSTNAME%%"$delim"*}"
    local label="$name"
    [[ "${__gpy_is_ssh:-0}" == "1" && -n "${__icon_hostname:-}" ]] && label="$__icon_hostname $name"

    __gpy_prompt_escape "$label" label
    __gpy_section_standalone "${__color_hostname_bg:-}" "${__color_hostname_fg:-}" \
        "$label" "$is_last" "$is_first"
}
