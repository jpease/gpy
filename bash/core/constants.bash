# Prompt defaults; the agent's theme export overrides these.
__prompt_color="${__prompt_color:-green}"

__icon_prompt="${__icon_prompt:-❯}"

__icon_root_prompt="${__icon_root_prompt:-!❯!}"
__root_prompt_color="${__root_prompt_color:-red}"

__enabled_segments="${__enabled_segments:-directory git}"

# Tightens the nc fallback below nc's integer-second -w.
GPY_IPC_TIMEOUT_MS="${GPY_IPC_TIMEOUT_MS:-150}"

# Budget when the prompt has nothing to show for a segment (a cold git cache,
# a directory not yet rendered); never shorter than GPY_IPC_TIMEOUT_MS.
GPY_IPC_COLD_TIMEOUT_MS="${GPY_IPC_COLD_TIMEOUT_MS:-500}"

# Entries older than this trigger a background refresh (self-heal for missed pushes).
GPY_GIT_INSTANT_CACHE_TTL_SECONDS="${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}"

GPY_LANGUAGE_CACHE_TTL_SECONDS="${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}"

# Background a segment leaves for the next chevron. Pre-theme default; the
# agent's `theme export` redefines it. With OUT_VAR, writes via `printf -v`
# so the render loop never forks.
__gpy_segment_bg() {
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

# Returned by request wrappers that forked the oneshot fallback, so a dead
# daemon costs at most one oneshot fork per render.
# shellcheck disable=SC2034
GPY_SEG_STATUS_ONESHOT=3
