# bash/segments/clock.bash

# Build the strftime(3) spec for the clock's active configuration (12h/24h,
# leading zero, seconds), for interpolation into bash's `\D{…}` prompt token.
#
# Mirrors fish's `__gpy_clock_date_format` and the agent's
# `ClockResolver::time_spec`, which is the authority whenever the agent
# renders. This copy exists for the fallback path below; the `%-I`/`%-H`
# no-pad forms match what fish produces after its `string trim`.
__gpy_clock_time_spec() {
    local time_format="${__time_format:-12}"
    local leading_zero="${__clock_show_leading_zero:-0}"
    local show_seconds="${__clock_show_seconds:-0}"

    local hour
    if [[ "$time_format" == "24" ]]; then
        if [[ "$leading_zero" == "1" ]]; then hour="%H"; else hour="%-H"; fi
    elif [[ "$leading_zero" == "1" ]]; then
        hour="%I"
    else
        hour="%-I"
    fi

    local spec="${hour}:%M"
    [[ "$show_seconds" == "1" ]] && spec="${spec}:%S"
    [[ "$time_format" != "24" ]] && spec="${spec} %p"
    printf '%s' "$spec"
}

# is_last/is_first arrive already as "true"/"" (#613: init.bash's dispatch-loop
# convention); prev_bg is threaded by the render loop for the opening chevron.
__gpy_segment_clock() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    # Agent-rendered whenever the theme sets `[segments.clock].format`, like
    # every other segment. The agent owns the powerline caps, the delimiter
    # colors and the time format; the response embeds bash's `\D{…}` token so
    # the clock keeps ticking without an IPC call per second.
    if declare -F __gpy_request_clock >/dev/null 2>&1; then
        local rendered
        rendered="$(__gpy_request_clock "$is_last" "$prev_bg" "$is_first")"
        if [[ -n "$rendered" ]]; then
            echo "$rendered"
            return 0
        fi
    fi

    # Fallback: no clock template in the theme, or the agent is unreachable.
    # Renders an uncapped block — bash has no local powerline-cap renderer —
    # but still honors the configured time format so a fallback clock does not
    # silently disagree with fish about what "12-hour, no seconds" means.
    local bg="${__color_clock_bg:-cyan}"
    local fg="${__color_clock_fg:-black}"
    local icon="${__icon_clock:-}"

    # Convert colors to ANSI codes. `transparent` (e.g. the flat Starship preset)
    # maps to the terminal default (SGR 49 bg / 39 fg) so the clock blends in
    # instead of falling through to the `*)` fallback color.
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

    echo "\[\033[${bg_code};${fg_code}m\] ${icon} \D{$(__gpy_clock_time_spec)} \[\033[0m\]"
}
