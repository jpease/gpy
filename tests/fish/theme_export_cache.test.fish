#!/usr/bin/env fish
# Test: __gpy_load_theme sources the cache file without spawning the agent binary

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/init.fish"

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

# --- Setup: fake XDG_CACHE_HOME and a theme export cache ---
set -l tmp_dir (mktemp -d)
set -gx XDG_CACHE_HOME "$tmp_dir/cache"
mkdir -p "$XDG_CACHE_HOME/gpy"

set -l cache_file "$XDG_CACHE_HOME/gpy/theme-export.fish"

# Write a fake theme export
printf 'set -g GPY_TEST_THEME_LOADED yes\n' > "$cache_file"

# __gpy_theme_export_cache_path must return the correct path
set -l got_path (__gpy_theme_export_cache_path)
if test "$got_path" = "$cache_file"
    check "__gpy_theme_export_cache_path returns XDG-based path" pass
else
    check "__gpy_theme_export_cache_path returns XDG-based path (got: $got_path)" fail
end

# __gpy_load_theme should source the cache file and set the variable
set -e GPY_TEST_THEME_LOADED
__gpy_load_theme >/dev/null 2>/dev/null

if test "$GPY_TEST_THEME_LOADED" = yes
    check "__gpy_load_theme sources cache without spawning binary" pass
else
    check "__gpy_load_theme sources cache without spawning binary" fail
end

# When the cache exists again, verify re-sourcing picks up updated content
printf 'set -g GPY_TEST_THEME_V2 updated\n' > "$cache_file"
set -e GPY_TEST_THEME_V2

__gpy_load_theme >/dev/null 2>/dev/null
if test "$GPY_TEST_THEME_V2" = updated
    check "__gpy_load_theme re-sources updated cache content" pass
else
    check "__gpy_load_theme re-sources updated cache content" fail
end

# Cleanup
rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count theme export cache tests passed"
