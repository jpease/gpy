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

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.zsh's dispatch-loop
    # convention, matching __gpy_request_username's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__username_format:-}" ]]; then
        __gpy_request_username "${USER:-}" "$is_last" "$prev_bg"
        return
    fi

    # Pure-zsh path: optional icon + zsh prompt-escape colors (mirror hostname.zsh).
    local name=${USER:-}
    local icon=${__icon_username:-""}
    local label=$name
    [[ -n "$icon" ]] && label="$icon $name"

    local bg=${__color_username_bg:-red}
    local fg=${__color_username_fg:-white}
    [[ $bg == transparent ]] && bg=default
    [[ $fg == transparent ]] && fg=default
    echo "%K{$bg}%F{$fg} $label %f%k"
}
