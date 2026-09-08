# zsh/segments/status.zsh

function __gpy_segment_status() {
    local last_status=$1

    # Default icons and colors
    local icon_ok=${__icon_status_ok:-"✔"}
    local icon_fail=${__icon_status_fail:-"✖"}
    # Fallback colors if not set by theme
    local color_ok=${__color_status_ok:-green}
    local color_fail=${__color_status_fail:-red}

    if [[ $last_status -eq 0 ]]; then
        echo "%F{$color_ok}$icon_ok%f"
    else
        echo "%F{$color_fail}$icon_fail%f"
    fi
}
