# zsh/core/constants.zsh

# Icon Theme: Nerd
typeset -g __gpy_icon_nerd_git_new="󰿞"
typeset -g __gpy_icon_nerd_git_staged="󱓞"
typeset -g __gpy_icon_nerd_git_unstaged="󰇷"
typeset -g __gpy_icon_nerd_git_untracked="?"
typeset -g __gpy_icon_nerd_git_stash="󰋀"
typeset -g __gpy_icon_nerd_git_clean="󰄬"
typeset -g __gpy_icon_nerd_git_ahead="󰁞"
typeset -g __gpy_icon_nerd_git_behind="󰁆"
typeset -g __gpy_icon_nerd_git_diverged="󰩋"
typeset -g __gpy_icon_nerd_lock="🔒"
typeset -g __gpy_icon_nerd_status_ok="✔"
typeset -g __gpy_icon_nerd_status_fail="✖"
typeset -g __gpy_icon_nerd_duration="󰑧"
typeset -g __gpy_icon_nerd_clock=""
typeset -g __gpy_icon_nerd_prompt="❯"

# Defaults (overridden by agent theme export)
typeset -g __gpy_ui_prompt_icon="❯"
typeset -g __prompt_color="green"
typeset -g __icon_prompt="❯"
typeset -g __icon_root_prompt="!❯!"
typeset -g __root_prompt_color="red"

# Initialize segments array
typeset -g -a __enabled_segments=()

# IPC round-trip bound (ms); __gpy_ms_to_secs rejects a non-numeric override.
typeset -g GPY_IPC_TIMEOUT_MS=${GPY_IPC_TIMEOUT_MS:-150}

# Budget when the prompt has nothing to show for a segment (a cold git cache,
# a directory not yet rendered); never shorter than GPY_IPC_TIMEOUT_MS.
typeset -g GPY_IPC_COLD_TIMEOUT_MS=${GPY_IPC_COLD_TIMEOUT_MS:-500}

# Stale git cache entries are still served but trigger a background refresh,
# so a missed agent repaint push self-heals within this bound.
typeset -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS=${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}

typeset -g GPY_LANGUAGE_CACHE_TTL_SECONDS=${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}

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
        hostname) __gpy_sbg="${__color_hostname_bg:-}" ;;
        username)
            if [[ "${__gpy_is_root:-0}" == "1" ]]; then
                __gpy_sbg="${__color_username_bg:-}"
            else
                __gpy_sbg="${__color_username_user_bg:-}"
            fi
            ;;
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
typeset -g GPY_SEG_STATUS_ONESHOT=3

# The agent's escaping sends data `%` as `%%` and cannot know prompt_bang's state.
setopt prompt_subst prompt_percent no_prompt_bang
