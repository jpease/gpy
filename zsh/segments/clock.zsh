# strftime spec for zsh's `%D{…}`; mirrors fish and the agent's ClockResolver::time_spec.
# With OUT_VAR, assigns fork-free instead of printing.
function __gpy_clock_time_spec() {
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

    local __spec="${hour}:%M"
    [[ "$show_seconds" == "1" ]] && __spec="${__spec}:%S"
    [[ "$time_format" != "24" ]] && __spec="${__spec} %p"
    if [[ -n "${1:-}" ]]; then
        printf -v "$1" '%s' "$__spec"
    else
        printf '%s' "$__spec"
    fi
}

# Args arrive as "true"/"" (init.zsh dispatch convention).
function __gpy_segment_clock() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

# Agent path embeds zsh's live `%D{...}` token so the clock ticks without IPC.
    if [[ -n "${__clock_format:-}" ]] && (( $+functions[__gpy_request_clock] )); then
        local rendered
        rendered=$(__gpy_request_clock "$is_last" "$prev_bg" "$is_first")
        if [[ -n "$rendered" ]]; then
            print -r -- "$rendered"
            return 0
        fi
    fi

# Local fallback: no clock template, or agent unreachable.
    local spec pad
    __gpy_clock_time_spec spec
    __gpy_prompt_escape "${__prompt_time_pad- }" pad
    __gpy_section_standalone "${__color_clock_bg:-}" "${__color_clock_fg:-}" \
        "${pad}%D{${spec}}" "$is_last" "$is_first"
}
