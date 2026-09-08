# zsh/segments/clock.zsh

function __gpy_segment_clock() {
    local bg=${__color_clock_bg:-cyan}
    local fg=${__color_clock_fg:-black}
    local icon=${__icon_clock:-""}

    # `transparent` (e.g. the flat Starship preset) maps to zsh's `default` color
    # keyword so the clock blends into the terminal background instead of emitting
    # an unrecognized `%K{transparent}` color name.
    [[ $bg == transparent ]] && bg=default
    [[ $fg == transparent ]] && fg=default

    # Zsh prompt expansion: %D{...} formats time
    # %D{%H:%M:%S} = HH:MM:SS format
    echo "%K{$bg}%F{$fg} $icon %D{%H:%M:%S} %f%k"
}
