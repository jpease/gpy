# zsh/segments/hostname.zsh

function __gpy_segment_hostname_detect() {
    [[ "${__gpy_is_ssh:-0}" == "1" || "${__hostname_show_always:-0}" == "1" ]]
}

function __gpy_segment_hostname() {
    local is_last=${1:-}
    local prev_bg=${2:-}

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.zsh's dispatch-loop
    # convention, matching __gpy_request_hostname's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__hostname_format:-}" ]]; then
        __gpy_request_hostname "$HOST" "$is_last" "$prev_bg" "${__gpy_is_ssh:-0}"
        return
    fi

    # Pure-zsh path: trim + optional icon + zsh prompt-escape colors (mirror clock.zsh).
    # The icon is Starship's ssh_symbol: drawn only in SSH sessions (#826).
    # NOTE: use the NO-COLON form `${var-.}` so an EXPLICIT empty trim_at ("no trim")
    # is preserved; the colon form `${var:-.}` would wrongly collapse "" to "." and trim.
    local delim=${__hostname_trim_at-.}
    local name=$HOST
    [[ -n "$delim" ]] && name=${HOST%%"$delim"*}   # trim at first delim; empty delim = no trim
    local icon=""
    [[ "${__gpy_is_ssh:-0}" == "1" ]] && icon=${__icon_hostname:-""}
    local label=$name
    [[ -n "$icon" ]] && label="$icon $name"

    local bg=${__color_hostname_bg:-black}
    local fg=${__color_hostname_fg:-white}
    [[ $bg == transparent ]] && bg=default
    [[ $fg == transparent ]] && fg=default

    # Escape literal % so prompt_subst doesn't reinterpret a %-containing
    # hostname as a prompt escape sequence (#324).
    local escaped_label=${label//\%/%%}
    print -r -- "%K{$bg}%F{$fg} $escaped_label %f%k"
}
