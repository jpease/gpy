# bash/segments/hostname.bash

__gpy_segment_hostname_detect() {
    [[ "${__gpy_is_ssh:-0}" == "1" || "${__hostname_show_always:-0}" == "1" ]]
}

__gpy_segment_hostname() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"

    # Agent-resolved path when a theme set a Starship-compatible format.
    # is_last arrives already as "true"/"" (#613: init.bash's dispatch-loop
    # convention, matching __gpy_request_hostname's own contract) -- forward
    # it directly, no conversion needed.
    if [[ -n "${__hostname_format:-}" ]]; then
        __gpy_request_hostname "$HOSTNAME" "$is_last" "$prev_bg"
        return
    fi

    # Pure-bash path: trim + optional icon + inline ANSI (mirror clock.bash).
    # NOTE: `${var-default}` (no colon) so an *unset* var defaults to "." but an
    # explicit empty string (the "no trim" contract) is preserved rather than
    # being collapsed to the default by the colon-form `${var:-default}`.
    local delim="${__hostname_trim_at-.}"
    local name="$HOSTNAME"
    [[ -n "$delim" ]] && name="${HOSTNAME%%"$delim"*}"
    local icon="${__icon_hostname:-}"
    local label="$name"
    [[ -n "$icon" ]] && label="$icon $name"

    local bg="${__color_hostname_bg:-black}"
    local fg="${__color_hostname_fg:-white}"

    # Convert colors to ANSI codes. `transparent` (e.g. the flat Starship preset)
    # maps to the terminal default (SGR 49 bg / 39 fg) so the hostname segment
    # blends in instead of falling through to the `*)` fallback color.
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
        *) bg_code="46" ;;
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
        *) fg_code="30" ;;
    esac

    echo "\[\033[${bg_code};${fg_code}m\] ${label} \[\033[0m\]"
}
