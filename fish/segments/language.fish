function segment_language_detect
    if test "$__gpy_cfg_language_enabled" = 0
        return 1
    end

    # A memoized .git root is itself a marker: skip the walk.
    set -l memoized_root (__gpy_memoized_git_root $PWD)
    if test -n "$memoized_root"
        return 0
    end

    # Markers come from the agent's `theme export`; until then defer to the request path.
    set -q __gpy_lang_marker_files[1]; or return 0

    # Cheap pre-filter so non-project directories fire no IPC.
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

# Cold miss always omits: fire the refresh here, before positions are assigned.
function segment_language_omit
    __gpy_instant_cache_present lang $PWD; and return 1
    __gpy_maybe_refresh lang $PWD lang "" "" ""
    return 0
end

function segment_language_render --argument-names is_last is_first
    # Seed missing args as "" (see fish_prompt.fish's dispatch loop).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    set -l root $PWD

    set -l prev_bg $__gpy_last_segment_bg

    # Exit code: 1 = miss; stale/variant bits read via __gpy_cache_status_*.
    set -l cache_suffix (__gpy_cache_variant_suffix lang $is_last $is_first)
    set -l rendered_output (__gpy_read_instant_cache $cache_suffix "$root" $prev_bg | string collect)
    set -l cache_status $pipestatus[1]

    if test "$cache_status" -eq 1
        # Entries exist only for other prev_bg contexts; refresh and wait for repaint.
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
    end

    printf '%s' "$rendered_output"

    set -g __gpy_last_segment_bg $__color_language_bg

    # Variant fallback: background refresh only, so language never blocks; throttled per prev_bg token.
    if __gpy_cache_status_variant $cache_status
        set -l token (__gpy_prev_bg_token "$prev_bg")
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first" "$token"
    end

    # Stale: __gpy_maybe_refresh bypasses the instant cache, which __gpy_request would short-circuit on.
    if __gpy_cache_status_stale $cache_status
        __gpy_maybe_refresh lang "$root" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
    end
end
