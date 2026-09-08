# bash/segments/clock.bash

__gpy_segment_clock() {
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

    # Use \t for 24-hour HH:MM:SS format
    echo "\[\033[${bg_code};${fg_code}m\] ${icon} \t \[\033[0m\]"
}
