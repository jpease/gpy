# zsh/segments/language.zsh

function __gpy_segment_language_detect() {
    [[ "${GPY_LANGUAGE_ENABLED:-1}" != "0" ]] || return 1

    # Walk upward from $PWD looking for project markers, mirroring the Fish
    # ancestor walk (#128) so the segment stays visible in project
    # subdirectories instead of only at the project root (#174). Pure
    # parameter expansion — no external process in the per-prompt path.
    # Marker list MUST stay aligned with fish/segments/language.fish.
    local dir="$PWD"
    while [[ -n "$dir" ]]; do
        if [[ -e "$dir/.git" || -f "$dir/Cargo.toml" || -f "$dir/package.json" \
            || -f "$dir/requirements.txt" || -f "$dir/Pipfile" \
            || -f "$dir/pyproject.toml" || -f "$dir/go.mod" \
            || -f "$dir/Gemfile" || -f "$dir/mix.exs" || -f "$dir/pom.xml" \
            || -f "$dir/build.gradle" || -f "$dir/build.gradle.kts" ]]; then
            return 0
        fi
        [[ "$dir" == "/" ]] && break
        dir="${dir%/*}"
        [[ -z "$dir" ]] && dir="/"
    done
    return 1
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
        # 2. Cold miss: trigger background refresh, show nothing until the agent repaints
        __gpy_trigger_data_refresh "lang" "$PWD" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
        return 0
    fi

    echo "$response"

    # 3. Stale hit (bit 2 set: status 2 or 6): serve above, refresh in background
    if (( (cache_status & 2) != 0 )); then
        __gpy_trigger_data_refresh "lang" "$PWD" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
    fi
}
