function segment_git_detect
    if set -q GPY_GIT_ENABLED; and test "$GPY_GIT_ENABLED" = 0
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
                set -g __gpy_last_segment_bg $__color_git_clean_bg
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
            set -l synced (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS | string collect)
            if test -n "$synced"
                printf '%s' "$synced"
                set -g __gpy_last_segment_bg $__color_git_clean_bg
                return 0
            end
        end
    end

    # `test -S`, not __gpy_socket_ready: its connect probe can block up to 1s.
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
            else
                printf '%s' "$rendered_output"
            end
        end
    end

    # Clean bg approximates the chevron colour; exact state is unknown here.
    set -g __gpy_last_segment_bg $__color_git_clean_bg

    return 0
end
