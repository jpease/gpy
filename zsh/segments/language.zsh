# zsh/segments/language.zsh

function __gpy_segment_language_detect() {
    [[ "${GPY_LANGUAGE_ENABLED:-1}" != "0" ]] || return 1

    # Walk upward from $PWD looking for project markers, mirroring the Fish
    # ancestor walk (#128) so the segment stays visible in project
    # subdirectories instead of only at the project root (#174). Pure
    # parameter expansion — no external process in the per-prompt path.
    # The marker list is the agent's own, exported by `theme export` as
    # __gpy_lang_marker_files (#785): no hand-kept list here to drift. If it
    # has not been exported yet (agent never ran), defer to the request path,
    # which the oneshot budget already bounds.
    (( ${#__gpy_lang_marker_files[@]} )) || return 0
    local dir="$PWD" marker
    while [[ -n "$dir" ]]; do
        [[ -e "$dir/.git" ]] && return 0
        for marker in "${__gpy_lang_marker_files[@]}"; do
            [[ -f "$dir/$marker" ]] && return 0
        done
        [[ "$dir" == "/" ]] && break
        dir="${dir%/*}"
        [[ -z "$dir" ]] && dir="/"
    done
    return 1
}

# Consulted by __gpy_segment_would_render after the detect above passes, before
# any position is assigned: returns 0 when this render is certain to print
# nothing, so the segment never takes a first/last position it cannot fill
# (#766). A cold miss (no instant-cache entry for this project in any variant)
# always omits, so fire the background refresh __gpy_segment_language would
# have fired here instead. It carries no position or prev_bg: the agent writes
# every variant, plus the context-free `.none` entry, and repaints.
function __gpy_segment_language_omit() {
    __gpy_instant_cache_present lang "$PWD" && return 1
    __gpy_trigger_data_refresh "lang" "$PWD" "" "" >/dev/null 2>&1 &!
    return 0
}

function __gpy_segment_language() {
    # is_last/is_first arrive already as "true"/"" (#613: init.zsh's
    # dispatch-loop convention), so no per-segment conversion is needed here.
    local is_last=$1
    # prev_bg: previous segment's background, threaded by the render loop so the
    # agent can color the opening powerline chevron (fg:prev_bg).
    local prev_bg=${2:-}
    local is_first=${3:-}
    local cache_suffix
    cache_suffix=$(__gpy_cache_variant_suffix lang "$is_last" "$is_first")

    # 1. Try reading from instant cache (serve-stale: always serve if exists).
    # Status is carried entirely by the exit code (#614: 1 = miss, bit 2 =
    # stale -- see __gpy_read_instant_cache's doc comment) instead of a
    # `__gpy_${suffix}_stale_$$` temp file.
    local response
    response=$(__gpy_read_instant_cache "$cache_suffix" "$PWD" "$prev_bg")
    local cache_status=$?

    if [[ $cache_status -eq 1 ]]; then
        # 2. Cold miss: trigger background refresh, show nothing until the agent repaints.
        # __gpy_segment_language_omit already dropped a plain cold miss before
        # positions were assigned (#766); this still catches entries that
        # exist only for other prev_bg contexts (no `.none`), where the
        # previous segment keeps its not-last form until the agent repaints.
        __gpy_trigger_data_refresh "lang" "$PWD" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
        return 0
    fi

    print -r -- "$response"

    # 3. Stale hit (bit 2 set: status 2 or 6): serve above, refresh in background
    if (( (cache_status & 2) != 0 )); then
        __gpy_trigger_data_refresh "lang" "$PWD" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
    fi
}
