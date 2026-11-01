function segment_git_detect
    if test "$__gpy_cfg_git_enabled" = 0
        return 1
    end

    # Reuses the render-scoped memoized git root shared with the cache read.
    test -n "$(__gpy_memoized_git_root $PWD)"
end

# Returns 0 when the render must print nothing (agent down, cold miss), so git takes no position.
function segment_git_omit
    not __gpy_agent_enabled; and return 1
    not __gpy_instant_cache_present git $PWD; and not test -S (__gpy_ipc_endpoint)
end

# Serve-stale render from the instant cache; bounded sync IPC on cold miss.
function segment_git_render --argument-names is_last is_first
    # Seed missing args as "" (see fish_prompt.fish's dispatch loop).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    set -l root $PWD

    set -l cache_suffix (__gpy_cache_variant_suffix git $is_last $is_first)

    set -l prev_bg $__gpy_last_segment_bg

    # Exit code: 1 = miss; stale/variant bits read via __gpy_cache_status_*.
    set -l rendered_output (__gpy_read_instant_cache $cache_suffix "$root" $prev_bg | string collect)
    set -l cache_status $pipestatus[1]

    if test "$cache_status" -eq 1
        # Agent-free mode renders through oneshot.
        if not __gpy_agent_enabled
            set -l fresh (__gpy_oneshot_fallback git "$root" ansi "$is_last" "$is_first" | string collect)
            if test -n "$fresh"
                printf '%s' "$fresh"
                set -g __gpy_last_segment_bg $__color_git_clean_bg
            end
            return 0
        end
        # Cold miss: bounded IPC-only query (oneshot is unbounded); instant omit if the socket is absent.
        # Nothing to show yet, so it may wait the longer cold budget.
        __gpy_register_with_agent >/dev/null 2>&1
        if test -S (__gpy_ipc_endpoint)
            set -l payload (__gpy_build_data_payload git "$root" ansi "$is_last" "$prev_bg" "$is_first")
            set -l synced (__gpy_ipc_send $payload (__gpy_ipc_cold_timeout_ms) | string collect)
            if test -n "$synced"
                # The agent writes its own instant cache while handling this request.
                printf '%s' "$synced"
                set -g __gpy_last_segment_bg (__gpy_git_segment_bg "$root")
                return 0
            end
        end

        # Omit and refresh in the background; the agent repaints via SIGURG.
        __gpy_maybe_refresh git "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    end

    # Served `.none` entry: chevron colour is wrong, so re-query with the real prev_bg.
    if __gpy_cache_status_variant $cache_status
        if test -S (__gpy_ipc_endpoint)
            set -l payload (__gpy_build_data_payload git "$root" ansi "$is_last" "$prev_bg" "$is_first")
            set -l synced (__gpy_ipc_send $payload (__gpy_ipc_timeout_ms) | string collect)
            if test -n "$synced"
                printf '%s' "$synced"
                set -g __gpy_last_segment_bg (__gpy_git_segment_bg "$root")
                return 0
            end
        end
    end

    # Colour the pill was drawn in, unless the oneshot below drew it agent-free.
    set -l agent_free 0

    # `test -S`, not a ping: a connect probe can block the prompt.
    if not __gpy_cache_status_stale $cache_status
        printf '%s' "$rendered_output"
    else
        if test -S (__gpy_ipc_endpoint)
            # Agent up: serve stale, background refresh repaints via SIGURG.
            printf '%s' "$rendered_output"
            __gpy_maybe_refresh git "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        else
            # Agent down: the refresh has no listener; stale beats empty if oneshot fails.
            set -l fresh (__gpy_oneshot_fallback git "$root" ansi "$is_last" "$is_first" | string collect)
            if test -n "$fresh"
                printf '%s' "$fresh"
                set agent_free 1
            else
                printf '%s' "$rendered_output"
            end
        end
    end

    # The agent records the pill's state colour; a stale file never colours an agent-free render.
    if test $agent_free -eq 1
        set -g __gpy_last_segment_bg $__color_git_clean_bg
    else
        set -g __gpy_last_segment_bg (__gpy_git_segment_bg "$root")
    end

    return 0
end
