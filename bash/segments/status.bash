# bash/segments/status.bash

__gpy_segment_status() {
    local last_status=$1

    # Default icons and colors
    local icon_ok="${__icon_status_ok:-✔}"
    local icon_fail="${__icon_status_fail:-✖}"
    local color_ok="${__color_status_ok:-green}"
    local color_fail="${__color_status_fail:-red}"

    # Convert color names to ANSI codes
    local color_code
    if [[ $last_status -eq 0 ]]; then
        case "$color_ok" in
            red) color_code="31" ;;
            green) color_code="32" ;;
            yellow) color_code="33" ;;
            blue) color_code="34" ;;
            magenta) color_code="35" ;;
            cyan) color_code="36" ;;
            white) color_code="37" ;;
            *) color_code="32" ;;
        esac
        printf '%s\n' "\[\033[${color_code}m\]${icon_ok}\[\033[0m\]"
    else
        case "$color_fail" in
            red) color_code="31" ;;
            green) color_code="32" ;;
            yellow) color_code="33" ;;
            blue) color_code="34" ;;
            magenta) color_code="35" ;;
            cyan) color_code="36" ;;
            white) color_code="37" ;;
            *) color_code="31" ;;
        esac
        printf '%s\n' "\[\033[${color_code}m\]${icon_fail}\[\033[0m\]"
    fi
}
