# zsh/segments/git.zsh

function __gpy_segment_git_detect() {
    [[ "${GPY_GIT_ENABLED:-1}" != "0" ]] || return 1
    __gpy_find_git_root "$PWD" >/dev/null
}

function __gpy_segment_git() {
    # is_last/is_first arrive already as "true"/"" (#613: init.zsh's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    local is_last=$1
    # prev_bg: previous segment's background, threaded by the render loop so the
    # agent can color the opening powerline chevron (fg:prev_bg).
    local prev_bg=${2:-}
    local is_first=${3:-}
    local cache_suffix
    cache_suffix=$(__gpy_cache_variant_suffix git "$is_last" "$is_first")

    local root="$PWD"

    # Read the instant cache directly (0ms) and read staleness off the exit
    # code (#614: 1 = miss, bit 2 = stale -- see __gpy_read_instant_cache's
    # doc comment) instead of a `__gpy_${suffix}_stale_$$` temp file.
    local rendered_output
    rendered_output=$(__gpy_read_instant_cache "$cache_suffix" "$root" "$prev_bg")
    local cache_status=$?

    if [[ $cache_status -eq 1 ]]; then
        # Cold miss: fall through to a full IPC request, with oneshot fallback.
        __gpy_request "git" "$root" "zsh-prompt" "$is_last" "$prev_bg" "$is_first"
        return
    fi

    [[ -z "$rendered_output" ]] && return 0

    # Fresh entry: print it and do nothing else.
    if (( (cache_status & 2) == 0 )); then
        printf '%s' "$rendered_output"
        return 0
    fi

    # Stale (bit 2 set: status 2 or 6). With the agent UP, serve the stale
    # value now and trigger a background IPC refresh so the agent updates the
    # instant cache and repaints via the SIGURG doorbell once the output changes. With the
    # agent DOWN that refresh reaches no listener and the stale value would be
    # shown indefinitely, in every directory with a cache entry (#639, the
    # #430 branch Fish already had): run a bounded foreground oneshot instead,
    # within the per-render oneshot budget, and fall back to the stale value
    # only if the oneshot produced nothing.
    if [[ -S "$(__gpy_ipc_endpoint)" ]]; then
        printf '%s' "$rendered_output"
        __gpy_trigger_data_refresh "git" "$root" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
        return 0
    fi

    local fresh oneshot_status
    fresh="$(__gpy_fallback_oneshot "git" "$root" "zsh-prompt" "$is_last" "$is_first")"
    oneshot_status=$?
    if [[ -n "$fresh" ]]; then
        printf '%s' "$fresh"
    else
        printf '%s' "$rendered_output"
    fi
    # Propagate the budget claim (GPY_SEG_STATUS_ONESHOT) to the render loop.
    return "$oneshot_status"
}
