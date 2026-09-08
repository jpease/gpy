# bash/segments/username.bash
#
# Starship `username`-module parity (#252): shows the effective username (e.g.
# `root`) as a styled prefix when running as root or under sudo, hidden
# otherwise. Opt-in — not in any theme's default enabled_segments.

__gpy_segment_username_detect() {
    [[ "${__gpy_is_root:-0}" == "1" || "${__gpy_is_sudo:-0}" == "1" || "${__username_show_always:-0}" == "1" ]]
}

__gpy_segment_username() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.bash's dispatch-loop
    # convention, matching __gpy_request_username's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__username_format:-}" ]]; then
        __gpy_request_username "${USER:-}" "$is_last" "$prev_bg"
        return
    fi

    # Pure-bash path: optional icon + inline ANSI (mirror hostname.bash).
    local name="${USER:-}"
    local icon="${__icon_username:-}"
    local label="$name"
    [[ -n "$icon" ]] && label="$icon $name"

    local bg="${__color_username_bg:-red}"
    local fg="${__color_username_fg:-white}"

    # Convert colors to ANSI codes. `transparent` maps to the terminal default
    # (SGR 49 bg / 39 fg) so the segment blends in instead of hitting `*)`.
    local bg_code fg_code
    case "$bg" in
        black) bg_code="40" ;;
        red) bg_code="41" ;;
        green) bg_code="42" ;;
        yellow) bg_code="43" ;;
        blue) bg_code="44" ;;
        magenta) bg_code="45" ;;
        cyan) bg_code="46" ;;
        white) bg_code="47" ;;
        transparent) bg_code="49" ;;
        *) bg_code="41" ;;
    esac

    case "$fg" in
        black) fg_code="30" ;;
        red) fg_code="31" ;;
        green) fg_code="32" ;;
        yellow) fg_code="33" ;;
        blue) fg_code="34" ;;
        magenta) fg_code="35" ;;
        cyan) fg_code="36" ;;
        white) fg_code="37" ;;
        transparent) fg_code="39" ;;
        *) fg_code="37" ;;
    esac

    echo "\[\033[${bg_code};${fg_code}m\] ${label} \[\033[0m\]"
}
