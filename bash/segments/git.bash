# bash/segments/git.bash

__gpy_segment_git_detect() {
    [[ "${GPY_GIT_ENABLED:-1}" != "0" ]] || return 1
    __gpy_find_git_root "$PWD" >/dev/null
}

# Consulted by __gpy_segment_would_render after the detect above passes, before
# any position is assigned: returns 0 when this render is certain to print
# nothing, so the segment never takes a first/last position it cannot fill
# (#766, #843). That is the cold miss with the agent down: no instant-cache
# entry for this repo in any variant and no socket, where __gpy_segment_git
# can only omit (its background refresh has no listener). Agent-free mode
# (GPY_AGENT_ENABLED=0) never omits: it renders through oneshot (#841).
# Mirrors fish's segment_git_omit.
__gpy_segment_git_omit() {
    __gpy_agent_enabled || return 1
    __gpy_instant_cache_present git "$PWD" && return 1
    [[ ! -S "$(__gpy_ipc_endpoint)" ]]
}

__gpy_segment_git() {
    # is_last/is_first arrive already as "true"/"" (#613: init.bash's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    local is_last="$1"
    # prev_bg: previous segment's background, threaded by the render loop so the
    # agent can color the opening powerline chevron (fg:prev_bg).
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    local cache_suffix
    cache_suffix="$(__gpy_cache_variant_suffix git "$is_last" "$is_first")"

    local root="$PWD"

    # Read the instant cache directly (0ms) and read staleness off the exit
    # code (#614: 1 = miss, bit 2 = stale -- see __gpy_read_instant_cache's
    # doc comment) instead of a `__gpy_${suffix}_stale_$$` temp file.
    local rendered_output
    rendered_output=$(__gpy_read_instant_cache "$cache_suffix" "$root" "$prev_bg")
    local cache_status=$?

    if [[ $cache_status -eq 1 ]]; then
        # Agent-free mode (GPY_AGENT_ENABLED=0, #841): there is no daemon to
        # query or wait for, so render through the oneshot fallback.
        if ! __gpy_agent_enabled; then
            __gpy_request "git" "$root" "bash-prompt" "$is_last" "$prev_bg" "$is_first"
            return
        fi
        # Cold miss (#434, mirrors fish/segments/git.fish). Register (a cheap
        # no-op once registered) so future watcher pushes reach this shell,
        # then attempt a BOUNDED synchronous IPC query so the FIRST prompt
        # shows correct git instead of omitting until an async repaint. Gated
        # on the socket existing inside __gpy_send_json, so a DOWN agent is an
        # instant omit (no IPC attempt, no hang); when up the query is bounded
        # by GPY_IPC_TIMEOUT_MS. Deliberately IPC-only: the oneshot fallback
        # (a full foreground `gpy-agent oneshot git`) is unbounded and would
        # violate the "graceful omit within budget" contract. The agent writes
        # its own instant cache while handling the request, so a success needs
        # no extra refresh.
        __gpy_register_with_agent >/dev/null 2>&1
        local synced
        if synced="$(__gpy_sync_data_request "git" "$root" "$is_last" "$prev_bg" "$is_first")"; then
            printf '%s' "$synced"
            return 0
        fi
        # Agent unreachable, or the query exceeded budget / returned empty:
        # omit the segment and refresh in the background; the agent repaints
        # via the SIGURG doorbell when the data lands. Omitting is a successful render.
        __gpy_maybe_refresh "git" "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    fi

    [[ -z "$rendered_output" ]] && return 0

    # Variant fallback (#436, bit 4): the read served the `.none` context-free
    # entry because no token-specific file exists yet for this render's real
    # prev_bg. The git content is right but the opening chevron was rendered
    # without prev_bg context, so its colour is wrong. Run the SAME bounded
    # synchronous IPC query as the cold-miss branch with the real prev_bg; the
    # agent's response also writes the token-specific file, so later renders
    # hit it directly. On agent-down or an empty/timed-out response, fall
    # through and serve the `.none` output below (a later render self-corrects).
    if __gpy_cache_status_variant "$cache_status"; then
        local synced_variant
        if synced_variant="$(__gpy_sync_data_request "git" "$root" "$is_last" "$prev_bg" "$is_first")"; then
            printf '%s' "$synced_variant"
            return 0
        fi
    fi

    # Fresh entry: print it and do nothing else.
    if (( (cache_status & 2) == 0 )); then
        printf '%s' "$rendered_output"
        return 0
    fi

    # Stale (bit 2 set: status 2 or 6). With the agent UP, serve the stale
    # value now and trigger a throttled background IPC refresh so the agent updates the
    # instant cache and repaints via the SIGURG doorbell once the output changes. With the
    # agent DOWN that refresh reaches no listener and the stale value would be
    # shown indefinitely, in every directory with a cache entry (#639, the
    # #430 branch Fish already had): run a bounded foreground oneshot instead,
    # within the per-render oneshot budget, and fall back to the stale value
    # only if the oneshot produced nothing. The cheap `test -S` stat is the
    # same liveness gate __gpy_send_json uses.
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
    # Propagate the budget claim (GPY_SEG_STATUS_ONESHOT) to the render loop.
    return "$oneshot_status"
}
