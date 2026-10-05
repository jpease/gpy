# bash/core/constants.bash
# Theme variables and constants

# Default prompt colors
__prompt_color="${__prompt_color:-green}"

# Default icons (Nerd Font)
__icon_prompt="${__icon_prompt:-❯}"

# Enabled segments (space-separated list)
__enabled_segments="${__enabled_segments:-directory git}"

# IPC round-trip bound in milliseconds (mirrors zsh/fish). Used to tighten the
# nc fallback's blocking window closer to the target instead of nc's coarse
# integer-second -w resolution (#324).
GPY_IPC_TIMEOUT_MS="${GPY_IPC_TIMEOUT_MS:-150}"

# Minimum time between per-prompt supervisor health checks (seconds). Rate
# limits __gpy_supervisor_check so a dead agent doesn't get a restart attempt
# forked on every single prompt (#324).
GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS="${GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS:-10}"

# Maximum consecutive restart attempts from the per-prompt supervisor check
# before backing off for the rest of the session.
GPY_SUPERVISOR_CHECK_MAX_ATTEMPTS="${GPY_SUPERVISOR_CHECK_MAX_ATTEMPTS:-3}"

# Git instant-cache TTL (seconds). This is the pull-based self-heal bound when
# an agent repaint push is missed: cached git output is still served instantly,
# but entries older than this flag a throttled background refresh.
GPY_GIT_INSTANT_CACHE_TTL_SECONDS="${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}"

# Language instant-cache TTL (seconds). Language output changes less frequently
# than git status, so stale language entries can self-heal on a longer bound.
GPY_LANGUAGE_CACHE_TTL_SECONDS="${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}"

# Background color a rendered segment leaves behind, so the next segment can
# draw its opening powerline chevron (fg:prev_bg). This is the pre-agent
# DEFAULT: `__gpy_load_theme` (init.bash) sources the agent's `theme export`
# output, which REDEFINES this function with the generated version built
# from the single Rust segment/bg-variable table
# (gpy-agent/src/theme/export.rs's SEGMENT_BG_VARS, #614) -- so this default
# only matters before that eval runs, or when gpy-agent itself is
# unavailable. Kept here (constants.bash, sourced before init.bash) rather
# than in init.bash, mirroring every other pre-theme default in this file
# (__prompt_color, __icon_prompt, ...).
#
# Two calling conventions:
#   __gpy_segment_bg SEGMENT           -> value on stdout (external callers)
#   __gpy_segment_bg SEGMENT OUT_VAR   -> value written into $OUT_VAR via
#                                          `printf -v` (fork-free: the hot
#                                          render-loop path in init.bash uses
#                                          this so a bg lookup never forks a
#                                          subshell, #342)
__gpy_segment_bg() {
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
# every `segment_output=$($segment_fn ..)` and folds it into the per-render
# __gpy_oneshot_used variable, so a dead daemon costs at most ONE oneshot fork
# per prompt render -- replacing the old predictable `.gpy_oneshot_used_$$`
# marker file under ${TMPDIR:-/tmp}.
# Read by init.bash, ipc.bash and segments/*.bash.
# shellcheck disable=SC2034
GPY_SEG_STATUS_ONESHOT=3
