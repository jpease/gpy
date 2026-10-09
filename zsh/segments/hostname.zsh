# zsh/segments/hostname.zsh

function __gpy_segment_hostname_detect() {
    [[ "${__gpy_is_ssh:-0}" == "1" || "${__hostname_show_always:-0}" == "1" ]]
}

function __gpy_segment_hostname() {
    local is_last=${1:-}
    local prev_bg=${2:-}
    local is_first=${3:-}

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.zsh's dispatch-loop
    # convention, matching __gpy_request_hostname's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__hostname_format:-}" ]]; then
        __gpy_request_hostname "$HOST" "$is_last" "$prev_bg" "${__gpy_is_ssh:-0}"
        return
    fi

    # Local path, drawn by the shared shell-side renderer so it matches fish's
    # pure-fish path for the same theme (#844). The icon is Starship's
    # ssh_symbol: drawn only in SSH sessions (#826).
    # NOTE: use the NO-COLON form `${var-.}` so an EXPLICIT empty trim_at ("no trim")
    # is preserved; the colon form `${var:-.}` would wrongly collapse "" to "." and trim.
    local delim=${__hostname_trim_at-.}
    local name=$HOST
    [[ -n "$delim" ]] && name=${HOST%%"$delim"*}   # trim at first delim; empty delim = no trim
    local label=$name
    [[ "${__gpy_is_ssh:-0}" == "1" && -n "${__icon_hostname:-}" ]] && label="$__icon_hostname $name"

    # Escapes a literal % (and the other prompt-expansion characters) so a
    # hostname cannot be reinterpreted as a prompt escape (#324, #677).
    __gpy_prompt_escape "$label" label
    __gpy_section_standalone "${__color_hostname_bg:-}" "${__color_hostname_fg:-}" \
        "$label" "$is_last" "$is_first"
}
