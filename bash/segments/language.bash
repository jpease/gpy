__gpy_segment_language_detect() {
    [[ "${GPY_LANGUAGE_ENABLED:-1}" != "0" ]] || return 1

    # Ancestor walk via parameter expansion only; markers come from the agent's export.
    # shellcheck disable=SC2154 # exported by `theme export`, unset until the agent has run
    [[ -n "${__gpy_lang_marker_files[*]-}" ]] || return 0
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

# Cold miss always omits: fire the refresh here, before positions are assigned.
__gpy_segment_language_omit() {
    __gpy_instant_cache_present lang "$PWD" && return 1
    __gpy_maybe_refresh "lang" "$PWD" lang "" "" ""
    return 0
}

__gpy_segment_language() {
    local is_last="$1"
    local prev_bg="${2:-}"
    local is_first="${3:-}"
    local cache_suffix
    __gpy_cache_variant_suffix lang "$is_last" "$is_first" cache_suffix

    # Exit code: 1 = miss, bit 2 = stale, bit 4 = variant fallback.
    local response
    __gpy_read_instant_cache "$cache_suffix" "$PWD" "$prev_bg" response
    local cache_status=$?

    if [[ $cache_status -eq 1 ]]; then
        # Entries exist only for other prev_bg contexts; refresh and wait for repaint.
        __gpy_maybe_refresh "lang" "$PWD" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
        return 0
    fi

    echo "$response"

    # Variant fallback: background refresh only, so language never blocks.
    if __gpy_cache_status_variant "$cache_status"; then
        local token
        __gpy_prev_bg_token "$prev_bg" token
        __gpy_maybe_refresh "lang" "$PWD" "$cache_suffix" "$is_last" "$prev_bg" "$is_first" "$token"
    fi

    if (( (cache_status & 2) != 0 )); then
        __gpy_maybe_refresh "lang" "$PWD" "$cache_suffix" "$is_last" "$prev_bg" "$is_first"
    fi
}
