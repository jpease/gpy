function segment_language_detect
    if set -q GPY_LANGUAGE_ENABLED; and test "$GPY_LANGUAGE_ENABLED" = 0
        return 1
    end

    # Reuse the render-scoped memoized git root (#342, core/ipc.fish): a .git
    # ancestor is itself one of the markers checked below, so if one was
    # already found this render (by segment_git_detect or the git segment's
    # cache read), this walk is guaranteed to return true too -- skip it
    # entirely rather than re-walking to confirm what we already know.
    set -l memoized_root (__gpy_memoized_git_root $PWD)
    if test -n "$memoized_root"
        return 0
    end

    # The marker list is the agent's own, exported by `theme export` as
    # __gpy_lang_marker_files (#785): no hand-kept list here to drift. If it
    # has not been exported yet (agent never ran), defer to the request path,
    # which the oneshot budget already bounds.
    set -q __gpy_lang_marker_files[1]; or return 0

    # Walk upward from $PWD looking for project markers.
    # The agent is authoritative; this is a cheap pre-filter so we don't
    # fire IPC requests in non-project directories.
    set -l dir $PWD
    while test -n "$dir"
        if test -e "$dir/.git"
            return 0
        end
        for marker in $__gpy_lang_marker_files
            if test -f "$dir/$marker"
                return 0
            end
        end
        set -l parent (path dirname -- "$dir")
        if test "$parent" = "$dir"
            break
        end
        set dir $parent
    end
    return 1
end

# Called by fish_prompt after segment_language_detect passes, before any
# position is assigned: returns 0 when this render is certain to print
# nothing, so the segment never takes a first/last position it cannot fill
# (#766). A cold miss (no instant-cache entry for this project in any variant)
# always omits, so fire the background refresh segment_language_render would
# have fired here instead. It carries no position or prev_bg: the agent writes
# every variant, plus the context-free `.none` entry, and repaints via SIGURG.
# Builtin-only, since it runs on every prompt in a project directory.
function segment_language_omit
    __gpy_instant_cache_present lang $PWD; and return 1
    __gpy_maybe_refresh lang $PWD lang "" "" ""
    return 0
end

# Renders language/tool versions. Tries to get data in this order:
# 1. Fast: from agent via IPC socket (if agent is running).
# 2. Medium: from a cache file (if not stale).
# 3. Slow: by running a one-shot gpy-agent command.
function segment_language_render --argument-names is_last is_first
    # A caller that passes fewer than 2 args (some tests call this with none,
    # to simulate a middle-of-prompt render) leaves the corresponding
    # --argument-names binding an EMPTY LIST, not an empty string -- normalize
    # so every unquoted use below always expands to exactly one word instead
    # of silently collapsing and shifting a later positional argument (#629-class).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    # Use current directory - agent will handle project root discovery
    set -l root $PWD

    # is_last/is_first arrive already as "true"/"" (#613: fish_prompt.fish's
    # dispatch-loop convention), so no per-segment conversion is needed here.

    # Snapshot prev_bg at render time so background jobs get the correct value.
    set -l prev_bg $__gpy_last_segment_bg

    # 1. Try reading from instant cache (0ms latency). Status is carried
    #    entirely by the exit code now (#612): 1 = miss, and
    #    __gpy_cache_status_stale/_variant read the freshness/variant-fallback
    #    bits off it (no more per-call staleness/variant-fallback globals to
    #    set-and-clear).
    set -l cache_suffix (__gpy_cache_variant_suffix lang $is_last $is_first)
    set -l rendered_output (__gpy_read_instant_cache $cache_suffix "$root" $prev_bg)
    set -l cache_status $status

    if test "$cache_status" -eq 1
        # 2. Cache miss - fire a throttled background refresh and render
        # nothing this time. The agent writes the cache and sends SIGURG when
        # done, causing a repaint.
        #
        # segment_language_omit already dropped a plain cold miss before
        # positions were assigned (#766). This branch still catches cache
        # entries that exist only for other prev_bg contexts (no `.none`):
        # the previous segment was drawn as not-last, so the line keeps a
        # trailing separator until the refresh lands (with the agent up) or
        # the agent returns.
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
    end

    # 3. Output cached content
    printf '%s' "$rendered_output"

    set -g __gpy_last_segment_bg $__color_language_bg

    # 3b. Variant fallback (#454, mirroring git's #436 correction). The read
    # served the `.none` context-free entry because no token-specific file
    # exists yet for this render's real prev_bg (first render at a new
    # powerline context) -- the language content is correct but the opening
    # chevron was rendered without prev_bg context, so its color is wrong.
    #
    # Unlike git (segment_git_render), this does NOT attempt a bounded
    # synchronous IPC query: language's cold-miss path above is deliberately
    # background-only to avoid blocking the prompt on slow language detection,
    # and that guarantee is worth preserving here too -- the visible defect is
    # milder than git's (only the chevron color is wrong; the language text
    # itself is already correct), so trading a possible per-prompt IPC wait
    # for a same-render fix isn't a good bargain. Instead, fire the SAME kind
    # of throttled background refresh as the cold-miss/stale paths, but with
    # the real prev_bg, so the agent's response writes the token-specific file
    # (write_language_variants) for the NEXT render to hit directly. This
    # render still shows the `.none` chevron once more -- a single
    # wrong-context frame, not an indefinite one.
    #
    # Throttled separately from the cold-miss/stale refresh above (keyed by
    # path+suffix+TOKEN, not just path+suffix, via __gpy_maybe_refresh's
    # throttle_key_extra): a recent miss/stale refresh for a DIFFERENT prev_bg
    # context must not starve this context's correction, and vice versa a
    # burst of renders at this SAME new context must still be capped to one
    # correction request per 500ms.
    if __gpy_cache_status_variant $cache_status
        set -l token (__gpy_prev_bg_token "$prev_bg")
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first" "$token"
    end

    # 4. If the cache is stale, trigger a background refresh while still showing
    # the cached result (no flicker). The agent will write a fresh cache file and
    # send SIGURG, causing a repaint with updated versions.
    #
    # Must bypass the instant cache (#458; __gpy_maybe_refresh always does).
    # `__gpy_request` reads it first and returns early on any hit, which is right
    # for rendering but would short-circuit on the very stale entry this refresh
    # exists to replace -- no IPC would ever leave the shell and the version could
    # stay stale indefinitely. The reply is discarded, and the agent's
    # write_language_variants rewrites all four is_last/is_first variants
    # regardless of which one was requested. Matches the variant-fallback path
    # above and the bash/zsh segments.
    if __gpy_cache_status_stale $cache_status
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
    end
end

# Language fallbacks removed - all operations now route through agent implementation
