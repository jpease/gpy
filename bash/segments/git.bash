__gpy_segment_git_detect() {
    [[ "${GPY_GIT_ENABLED:-1}" != "0" ]] || return 1
    local root
    __gpy_find_git_root "$PWD" root
}

# Returns 0 when the render must print nothing (agent down, cold miss), so git takes no position.
__gpy_segment_git_omit() {
    __gpy_agent_enabled || return 1
    __gpy_instant_cache_present git "$PWD" && return 1
    [[ ! -S "$(__gpy_ipc_endpoint)" ]]
}

__gpy_segment_git() {
    local is_last="$1"
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    local cache_suffix
    __gpy_cache_variant_suffix git "$is_last" "$is_first" cache_suffix

    local root="$PWD"

# Exit code: 1 = miss, bit 2 = stale, bit 4 = variant fallback.
    local rendered_output
    __gpy_read_instant_cache "$cache_suffix" "$root" "$prev_bg" rendered_output
    local cache_status=$?

    if [[ $cache_status -eq 1 ]]; then
        # Agent-free mode renders through oneshot.
        if ! __gpy_agent_enabled; then
            __gpy_request "git" "$root" "bash-prompt" "$is_last" "$prev_bg" "$is_first"
            return
        fi
        # Cold miss: bounded IPC-only query (oneshot is unbounded); instant omit if the socket is absent.
        # Nothing to show yet, so it may wait the longer cold budget.
        __gpy_register_with_agent >/dev/null 2>&1
        local synced cold_ms
        __gpy_cold_timeout_ms_into cold_ms
        if synced="$(__gpy_sync_data_request "git" "$root" "$is_last" "$prev_bg" "$is_first" "$cold_ms")"; then
            printf '%s' "$synced"
            return 0
        fi
        # Omit and refresh in the background; the agent repaints via SIGURG.
        __gpy_maybe_refresh "git" "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    fi

    [[ -z "$rendered_output" ]] && return 0

    # Served `.none` entry: chevron colour is wrong, so re-query with the real prev_bg.
    if __gpy_cache_status_variant "$cache_status"; then
        local synced_variant
        if synced_variant="$(__gpy_sync_data_request "git" "$root" "$is_last" "$prev_bg" "$is_first")"; then
            printf '%s' "$synced_variant"
            return 0
        fi
    fi

    if (( (cache_status & 2) == 0 )); then
        printf '%s' "$rendered_output"
        return 0
    fi

    # Stale with agent down: a background refresh has no listener, so run a bounded oneshot.
    if __gpy_agent_enabled && [[ -S "$(__gpy_ipc_endpoint)" ]]; then
        printf '%s' "$rendered_output"
        __gpy_maybe_refresh "git" "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    fi

    local fresh oneshot_status
    fresh="$(__gpy_fallback_oneshot "git" "$root" "bash-prompt" "$is_last" "$is_first")"
    oneshot_status=$?
    if [[ -n "$fresh" ]]; then
        printf '%s' "$fresh"
    else
        printf '%s' "$rendered_output"
    fi
    return "$oneshot_status"
}
