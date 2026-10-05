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
typeset -g __icon_root_prompt="#"
typeset -g __root_prompt_color="red"

# Initialize segments array
typeset -g -a __enabled_segments=()

# IPC Timeouts
typeset -g GPY_IPC_TIMEOUT_MS=150

# Git instant-cache TTL (seconds). This is the pull-based self-heal bound when
# an agent repaint push is missed: cached git output is still served instantly,
# but entries older than this flag a throttled background refresh.
typeset -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS=${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}

# Language instant-cache TTL (seconds). Language output changes less frequently
# than git status, so stale language entries can self-heal on a longer bound.
typeset -g GPY_LANGUAGE_CACHE_TTL_SECONDS=${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}

# Background color a rendered segment leaves behind, so the next segment can
# draw its opening powerline chevron (fg:prev_bg). This is the pre-agent
# DEFAULT: `__gpy_load_theme` (init.zsh) sources the agent's `theme export`
# output, which REDEFINES this function with the generated version built
# from the single Rust segment/bg-variable table
# (gpy-agent/src/theme/export.rs's SEGMENT_BG_VARS, #614) -- so this default
# only matters before that eval runs, or when gpy-agent itself is
# unavailable. Kept here (constants.zsh, sourced before init.zsh) rather
# than in init.zsh, mirroring every other pre-theme default in this file
# (__prompt_color, __icon_prompt, ...).
#
# Two calling conventions:
#   __gpy_segment_bg SEGMENT           -> value on stdout (external callers)
#   __gpy_segment_bg SEGMENT OUT_VAR   -> value written into $OUT_VAR via
#                                          `printf -v` (fork-free: the hot
#                                          render-loop path in init.zsh uses
#                                          this so a bg lookup never forks a
#                                          subshell, #342)
function __gpy_segment_bg() {
    local __gpy_sbg=""
    case "$1" in
        git) __gpy_sbg="${__color_git_clean_bg:-}" ;;
        language) __gpy_sbg="${__color_language_bg:-}" ;;
        directory) __gpy_sbg="${__color_directory_bg:-}" ;;
        duration) __gpy_sbg="${__color_duration_bg:-}" ;;
        clock) __gpy_sbg="${__color_clock_bg:-}" ;;
        hostname) __gpy_sbg="${__color_hostname_bg:-}" ;;
        username) __gpy_sbg="${__color_username_bg:-}" ;;
        *) __gpy_sbg="" ;;
    esac
    if [[ -n "$2" ]]; then
        printf -v "$2" '%s' "$__gpy_sbg"
    else
        printf '%s' "$__gpy_sbg"
    fi
}

# Distinct exit code a request wrapper (__gpy_request, __gpy_request_duration,
# __gpy_request_character) returns when it actually forked the oneshot
# fallback this call (#614). __gpy_render_prompt reads this off `$?` after
# every `seg_out=$(__gpy_segment_$seg ..)` and folds it into the per-render
# __gpy_oneshot_used variable, so a dead daemon costs at most ONE oneshot fork
# per prompt render -- replacing the old predictable `.gpy_oneshot_used_$$`
# marker file under ${TMPDIR:-/tmp}.
typeset -g GPY_SEG_STATUS_ONESHOT=3

# Ensure prompt substitution is enabled, and pin the prompt options the
# agent's zsh-prompt escaping assumes (#677): `%` escapes on (data `%` is sent
# as `%%`), and `prompt_bang` off (a `!` in data would otherwise become the
# history number; the escaping cannot know the option's state).
setopt prompt_subst prompt_percent no_prompt_bang
