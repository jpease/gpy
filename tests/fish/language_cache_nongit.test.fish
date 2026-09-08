#!/usr/bin/env fish
# Test: language instant cache is readable in non-Git project directories (#173)
# The agent keys the language cache by the canonical request path when no Git
# root exists (e.g. a package.json / Cargo.toml project with no .git). The shell
# reader must mirror that fallback so warm language caches are consumed instead
# of cold-missing forever. The Git cache must still require a Git root.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# --- Setup: a non-Git project directory (package.json, no .git) ---
set -l tmp_dir (mktemp -d)
set -g proj "$tmp_dir/no-git-project"
mkdir -p "$proj"
touch "$proj/package.json"

set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$tmp_dir/cache/gpy/instant-prompts"
mkdir -p "$cache_dir"

# Agent keys the cache by the canonical request path (no Git root).
set -l resolved (path resolve -- "$proj" 2>/dev/null)
if test -z "$resolved"
    set resolved $proj
end
set -l cache_key (__gpy_path_to_cache_key "$resolved")

set -g __gpy_prompt_now (date +%s)
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30

# --- Fresh language cache: served, and not flagged stale (no extra refresh) ---
set -l lang_file "$cache_dir/$cache_key.lang.none.ansi"
printf '\e[32m js 22.0.0 \e[0m' >"$lang_file"

set -l fresh (__gpy_read_instant_cache lang "$proj")
set -l fresh_status $status
if test "$fresh" = (printf '\e[32m js 22.0.0 \e[0m')
    check "fresh lang cache served in non-git project" pass
else
    check "fresh lang cache served in non-git project" fail
end
# #612: staleness is now the exit-code status (2/6), not a side-effect global.
if __gpy_cache_status_stale $fresh_status
    check "fresh warm read does not flag a refresh" fail
else
    check "fresh warm read does not flag a refresh" pass
end

# --- Stale language cache: still served, but flagged for background refresh ---
# Back-date well beyond the TTL.
touch -t 202001010000 "$lang_file" 2>/dev/null
set -l stale (__gpy_read_instant_cache lang "$proj")
set -l stale_status $status
if test "$stale" = (printf '\e[32m js 22.0.0 \e[0m')
    check "stale lang cache still served in non-git project" pass
else
    check "stale lang cache still served in non-git project" fail
end
if __gpy_cache_status_stale $stale_status
    check "stale read flags a background refresh" pass
else
    check "stale read flags a background refresh" fail
end

# --- Git cache must NOT use the path fallback (requires a real Git root) ---
set -l git_file "$cache_dir/$cache_key.git.none.ansi"
printf 'should-not-be-served' >"$git_file"
set -l git_out (__gpy_read_instant_cache git "$proj")
if test -z "$git_out"
    check "git cache miss in non-git project (no path fallback)" pass
else
    check "git cache miss in non-git project (no path fallback)" fail
end

# Cleanup
rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count non-git language cache tests passed"
