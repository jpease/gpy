function segment_git_detect
    if set -q GPY_DEBUG_SEGMENTS
        echo "[DEBUG] segment_git_detect: GPY_GIT_ENABLED=$GPY_GIT_ENABLED" >&2
    end

    if set -q GPY_GIT_ENABLED; and test "$GPY_GIT_ENABLED" = 0
        if set -q GPY_DEBUG_SEGMENTS
            echo "[DEBUG] segment_git_detect: Git disabled by config, returning 1" >&2
        end
        return 1
    end

    # Look for a .git marker in current or parent directories. Reuses the
    # render-scoped memoized git root (#342, core/ipc.fish) instead of walking
    # $PWD upward again -- the git segment's render path (via
    # __gpy_read_instant_cache) walks the same tree for its cache key, so this
    # shares that single walk rather than repeating it.
    if set -q GPY_DEBUG_SEGMENTS
        echo "[DEBUG] segment_git_detect: Searching for .git from $PWD" >&2
    end

    set -l root (__gpy_memoized_git_root $PWD)
    if test -n "$root"
        if set -q GPY_DEBUG_SEGMENTS
            echo "[DEBUG] segment_git_detect: Found .git at $root, returning 0" >&2
        end
        return 0
    end

    if set -q GPY_DEBUG_SEGMENTS
        echo "[DEBUG] segment_git_detect: No .git found, returning 1" >&2
    end
    return 1
end

# Called by fish_prompt after segment_git_detect passes, before any position is
# assigned: returns 0 when this render is certain to print nothing, so the
# segment never takes a first/last position it cannot fill (#766). That is the
# cold miss with the agent down: no instant-cache entry for this repo in any
# variant and no socket, where segment_git_render can only omit (and its
# background refresh has no listener). Builtin-only, since it runs on every
# prompt.
function segment_git_omit
    not __gpy_instant_cache_present git $PWD; and not test -S (__gpy_ipc_endpoint)
end

# Renders Git status from the instant-prompt cache (serve-stale, issue #160).
#
# A fresh entry is read from the agent-maintained instant cache (0ms, no IPC).
# A stale entry is displayed immediately and refreshed in the background; the
# agent repaints via the SIGURG doorbell. A cold miss with the agent up makes a
# synchronous IPC request bounded by GPY_IPC_TIMEOUT_MS; with the agent down it
# omits the segment. A stale entry with the agent down is replaced by a
# foreground `gpy-agent oneshot` render.
function segment_git_render --argument-names is_last is_first
    # A caller that passes fewer than 2 args (some tests call this with none,
    # to simulate a middle-of-prompt render) leaves the corresponding
    # --argument-names binding an EMPTY LIST, not an empty string -- normalize
    # so every unquoted use below always expands to exactly one word instead
    # of silently collapsing and shifting a later positional argument (#629-class).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    # Use current directory - agent keys the cache by git repository root.
    set -l root $PWD

    # Select the cache variant matching the delimiters (last/first vs not).
    # is_last/is_first arrive already as "true"/"" (#613: fish_prompt.fish's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    set -l cache_suffix (__gpy_cache_variant_suffix git $is_last $is_first)

    # Snapshot prev_bg at render time so background jobs get the correct value.
    set -l prev_bg $__gpy_last_segment_bg

    # 1. Read the instant cache (0ms). Status is carried entirely by the exit
    #    code now (#612): 1 = miss, and __gpy_cache_status_stale/_variant read
    #    the freshness/variant-fallback bits off it (no more per-call
    #    staleness/variant-fallback globals to set-and-clear).
    set -l rendered_output (__gpy_read_instant_cache $cache_suffix "$root" $prev_bg)
    set -l cache_status $status

    if test "$cache_status" -eq 1
        # 2. Cold miss (#434). Register (foreground, cheap no-op once
        #    registered) so future watcher pushes reach this shell, then
        #    attempt a BOUNDED synchronous IPC query so the FIRST prompt shows
        #    correct git instead of omitting until an async repaint. Gated on
        #    `test -S` so a DOWN agent is an instant omit (no IPC attempt, no
        #    hang); when up, __gpy_ipc_send is bounded by GPY_IPC_TIMEOUT_MS
        #    (~150ms), so a slow agent can't hang the prompt beyond budget.
        #    This is deliberately IPC-only: routing through the oneshot
        #    fallback (a full foreground `gpy-agent oneshot git`) is unbounded
        #    and would violate the "graceful omit within budget" contract. On
        #    any miss, fall back to today's omit + async refresh.
        __gpy_register_with_agent >/dev/null 2>&1
        if test -S (__gpy_ipc_endpoint)
            set -l payload (__gpy_build_data_payload git "$root" ansi "$is_last" "$prev_bg" "$is_first")
            set -l synced (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)
            if test -n "$synced"
                # The agent's git handler writes its own instant cache and
                # notifies other stale shells as part of handling this very
                # request (get_git_status -> publish_and_repaint), so no
                # extra background refresh is needed here on success.
                printf '%s' "$synced"
                set -g __gpy_last_segment_bg $__color_git_clean_bg
                return 0
            end
        end

        # Agent unreachable, or the query exceeded budget / returned empty:
        # keep today's behavior -- omit the segment and refresh in the
        # background. The agent repaints via SIGURG when the data lands.
        # Omitting output is a successful render, so always return 0.
        #
        # segment_git_omit already dropped the agent-down cold miss before
        # positions were assigned (#766). What still reaches this omit after
        # the previous segment was drawn as not-last, leaving a trailing
        # separator until the repaint: a bounded query that times out with
        # the agent up, and cache entries that exist only for other prev_bg
        # contexts (no `.none`) while the agent is down, which lasts until
        # the agent returns.
        __gpy_maybe_refresh git "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    end

    # 2b. Variant fallback (#436). The read served the `.none` context-free
    #     entry because no token-specific file exists yet for this render's
    #     real prev_bg (first render at a new powerline context) -- the git
    #     status content is correct but the opening chevron was rendered
    #     without prev_bg context, so its color is wrong. Try the SAME bounded
    #     synchronous IPC query as the cold-miss branch above, this time with
    #     the real prev_bg, so the FIRST render at this context shows the
    #     correct chevron instead of a wrong-color cycle. The agent's response
    #     also writes the token-specific file (write_git), so later renders at
    #     this context hit the exact-token file directly without this branch.
    #     On agent-down, or an empty/timed-out response, fall through and serve
    #     the `.none` rendered_output below exactly as before (safety net) --
    #     a later render self-corrects once a background refresh (or another
    #     shell's query) writes the token file.
    if __gpy_cache_status_variant $cache_status
        if test -S (__gpy_ipc_endpoint)
            set -l payload (__gpy_build_data_payload git "$root" ansi "$is_last" "$prev_bg" "$is_first")
            set -l synced (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)
            if test -n "$synced"
                printf '%s' "$synced"
                set -g __gpy_last_segment_bg $__color_git_clean_bg
                return 0
            end
        end
    end

    # 3. Serve the cached content (pre-rendered ANSI).
    #
    #    Fresh entry (not stale) is the hottest path: print it and do nothing
    #    else -- no socket probe, no added cost.
    #
    #    Stale entry: serve-stale-first (#160) shows the stale value and refreshes
    #    in the background, relying on the agent to SIGURG a repaint once it
    #    recomputes. That recovery only works while the agent is UP; with the
    #    agent DOWN the background refresh reaches no listener, so the stale value
    #    would be shown indefinitely -- the prompt strands on the old working-tree
    #    state (#430). Detect agent-down with the SAME cheap `test -S` socket stat
    #    __gpy_ipc_send uses as its liveness gate (NOT __gpy_socket_ready, which
    #    does a connect probe with up to a 1s timeout and would add prompt
    #    latency), and in that case run a bounded foreground `gpy-agent oneshot`
    #    to render the correct-but-slower result instead of the stale value.
    #
    #    AC2 / residual idle bound (by design, no periodic self-repaint tick):
    #    while the agent stays UP, a push/working-tree change missed by a fully
    #    idle shell (one that never renders a prompt) is only corrected on its
    #    next prompt render or the agent's ~45s reconcile push -- there is no
    #    timer that repaints an idle shell on its own. After an agent RESTART an
    #    idle shell still converges promptly via the .reregister doorbell
    #    (__gpy_doorbell_handler -> re-register + force-repaint); see ipc.fish.
    if not __gpy_cache_status_stale $cache_status
        printf '%s' "$rendered_output"
    else
        if test -S (__gpy_ipc_endpoint)
            # Agent up: serve stale now, refresh in the background (SIGURG
            # repaints iff the recomputed output changed). No flicker; the only
            # cost added over the fresh path is the single `test -S` stat above.
            printf '%s' "$rendered_output"
            __gpy_maybe_refresh git "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        else
            # Agent down: the background refresh has no listener, so serving the
            # stale value would strand the prompt on the old state. Render a
            # bounded foreground oneshot instead. If the oneshot also fails (e.g.
            # binary missing / throttle already claimed), fall back to the stale
            # value -- stale beats an empty segment.
            set -l fresh (__gpy_oneshot_fallback git "$root" ansi "$is_last" "$is_first")
            if test -n "$fresh"
                printf '%s' "$fresh"
            else
                printf '%s' "$rendered_output"
            end
        end
    end

    # Track this segment's bg so the next segment can render a powerline chevron.
    # Use clean bg as the best approximation — exact state is unknown at shell time.
    set -g __gpy_last_segment_bg $__color_git_clean_bg

    # A render that produced output (or correctly omitted it) is a success;
    # don't leak the background-refresh helper's exit status to the caller.
    return 0
end
