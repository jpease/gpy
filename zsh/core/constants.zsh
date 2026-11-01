# zsh/core/constants.zsh

# Defaults (overridden by agent theme export)
typeset -g __prompt_color="green"
typeset -g __icon_prompt="❯"
typeset -g __icon_root_prompt="!❯!"
typeset -g __root_prompt_color="red"

# The segments shown when no theme export loaded. Keep equal to
# DEFAULT_ENABLED_SEGMENTS in gpy-agent/src/config/defaults.rs;
# tests/fixtures/default_segments.txt pins this copy, the Bash and Fish ones
# and the agent's list to each other. Segments that cannot render without the
# agent omit themselves.
typeset -ga __gpy_fallback_segments=(clock duration language directory git)

# The theme export overwrites this; an export that sets it empty stays empty.
typeset -ga __enabled_segments=($__gpy_fallback_segments)

# Pre-theme default; `theme export` redefines it. With OUT_VAR the value is
# written via `printf -v` so the hot render loop never forks.
function __gpy_segment_bg() {
    local __gpy_sbg=""
    case "$1" in
        git) __gpy_sbg="${__color_git_clean_bg:-}" ;;
        language) __gpy_sbg="${__color_language_bg:-}" ;;
        directory) __gpy_sbg="${__color_directory_bg:-}" ;;
        duration) __gpy_sbg="${__color_duration_bg:-}" ;;
        clock) __gpy_sbg="${__color_clock_bg:-}" ;;
        status)
            if [[ "${__gpy_last_status:-0}" -eq 0 ]]; then
                __gpy_sbg="${__color_status_ok_bg:-green}"
            else
                __gpy_sbg="${__color_status_fail_bg:-red}"
            fi
            ;;
        *) __gpy_sbg="" ;;
    esac
    if [[ -n "$2" ]]; then
        printf -v "$2" '%s' "$__gpy_sbg"
    else
        printf '%s' "$__gpy_sbg"
    fi
}

# Returned by request wrappers when they forked the oneshot fallback; the
# render loop folds it in so a dead daemon costs one fork per render.
typeset -g __gpy_seg_status_oneshot=3

# Zsh expands PROMPT after precmd returns, so both options must stay global.
# prompt_bang is left as the user set it: __gpy_update_prompts doubles each
# data `!` when it is on.
setopt prompt_subst prompt_percent
