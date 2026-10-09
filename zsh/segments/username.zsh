# zsh/segments/username.zsh
#
# Starship `username`-module parity (#252): shows the effective username (e.g.
# `root`) as a styled prefix when running as root or under sudo, hidden
# otherwise. Opt-in — not in any theme's default enabled_segments.

function __gpy_segment_username_detect() {
    [[ "${__gpy_is_root:-0}" == "1" || "${__gpy_is_sudo:-0}" == "1" || "${__username_show_always:-0}" == "1" ]]
}

function __gpy_segment_username() {
    local is_last=${1:-}
    local prev_bg=${2:-}
    local is_first=${3:-}

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.zsh's dispatch-loop
    # convention, matching __gpy_request_username's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__username_format:-}" ]]; then
        __gpy_request_username "${USER:-}" "$is_last" "$prev_bg"
        return
    fi

    # Local path, drawn by the shared shell-side renderer so it matches fish's
    # pure-fish path for the same theme (#844).
    local label=${USER:-}
    [[ -n "${__icon_username:-}" ]] && label="$__icon_username ${USER:-}"

    __gpy_prompt_escape "$label" label
    __gpy_section_standalone "${__color_username_bg:-}" "${__color_username_fg:-}" \
        "$label" "$is_last" "$is_first"
}
