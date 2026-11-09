#!/usr/bin/env fish
# Test: __gpy_load_theme sources the cache file without spawning the agent binary

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/init.fish"

# --- Setup: fake XDG_CACHE_HOME and a theme export cache ---
set -l tmp_dir (mktemp -d)
set -gx XDG_CACHE_HOME "$tmp_dir/cache"
mkdir -p "$XDG_CACHE_HOME/gpy"

set -l cache_file "$XDG_CACHE_HOME/gpy/theme-export.fish"

# Write a fake theme export
printf '%s\n%s\n' 'set -g __gpy_export_version "1"' 'set -g GPY_TEST_THEME_LOADED yes' >"$cache_file"

# __gpy_theme_export_cache_path must return the correct path
set -l got_path (__gpy_theme_export_cache_path)
if test "$got_path" = "$cache_file"
    check "__gpy_theme_export_cache_path returns XDG-based path" pass pass
else
    check "__gpy_theme_export_cache_path returns XDG-based path (got: $got_path)" pass fail
end

# __gpy_load_theme should source the cache file and set the variable
set -e GPY_TEST_THEME_LOADED
__gpy_load_theme >/dev/null 2>/dev/null

if test "$GPY_TEST_THEME_LOADED" = yes
    check "__gpy_load_theme sources cache without spawning binary" pass pass
else
    check "__gpy_load_theme sources cache without spawning binary" pass fail
end

# When the cache exists again, verify re-sourcing picks up updated content
printf '%s\n%s\n' 'set -g __gpy_export_version "1"' 'set -g GPY_TEST_THEME_V2 updated' >"$cache_file"
set -e GPY_TEST_THEME_V2

__gpy_load_theme >/dev/null 2>/dev/null
if test "$GPY_TEST_THEME_V2" = updated
    check "__gpy_load_theme re-sources updated cache content" pass pass
else
    check "__gpy_load_theme re-sources updated cache content" pass fail
end

# --- No cache file: the agent is run, and its failure is reported ---
rm -f "$cache_file"
set -l stub_dir "$tmp_dir/bin"
mkdir -p "$stub_dir"
set -gx PATH $stub_dir $PATH
set -l stub "$stub_dir/gpy-agent"

function write_stub --argument-names stub body
    printf '#!/bin/sh\n%s\n' "$body" >$stub
    chmod +x $stub
end

write_stub $stub 'exit 1'
check "stub gpy-agent shadows any real binary" "$stub" (command -s gpy-agent)

__gpy_load_theme >/dev/null 2>&1
check "__gpy_load_theme returns 1 when theme export exits 1 without output" 1 $status

write_stub $stub 'echo "error: bad config" >&2; exit 1'
set -l err (GPY_DEBUG=1 __gpy_load_theme 2>&1 >/dev/null | string collect)
check "__gpy_load_theme logs the agent's first stderr line under GPY_DEBUG" 1 (string match -q '*gpy[init]: theme export failed: error: bad config*' -- "$err"; and echo 1; or echo 0)

write_stub $stub 'exit 0'
__gpy_load_theme >/dev/null 2>&1
check "__gpy_load_theme returns 1 when theme export exits 0 with no output" 1 $status

write_stub $stub 'echo "set -g __gpy_export_version \"1\""; echo "set -g GPY_TEST_THEME_FROM_AGENT yes"'
set -e GPY_TEST_THEME_FROM_AGENT
__gpy_load_theme >/dev/null 2>&1
check "__gpy_load_theme returns 0 for a successful export without a cache file" 0 $status
check "__gpy_load_theme sources a successful export without a cache file" yes "$GPY_TEST_THEME_FROM_AGENT"

# Cleanup
rm -rf "$tmp_dir"

test_finish "theme export cache tests"
