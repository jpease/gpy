#!/usr/bin/env fish

# Clear GPY language cache variables and instant prompt cache files.
# This is useful if you suspect the cache contains old rendered prompt data.

set -l cleared_count 0

# Clear all language cache variables (data, time, and version)
for var in (set -g | grep '^__gpy_lang_cache' | string replace -r ' .*' '')
    set -e $var
    set cleared_count (math $cleared_count + 1)
end

if test $cleared_count -gt 0
    echo "Cleared $cleared_count GPY language cache variables"
    echo "This includes cache data, timestamps, and format versions"
else
    echo "No GPY language cache variables found"
end

set -l cache_dir
if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"
    set cache_dir "$XDG_CACHE_HOME/gpy/instant-prompts"
else if set -q HOME
    set cache_dir "$HOME/.cache/gpy/instant-prompts"
end

set -l removed_files 0
if test -n "$cache_dir"; and test -d "$cache_dir"
    for file in "$cache_dir"/*.ansi
        if test -f "$file"
            rm -f "$file"
            set removed_files (math $removed_files + 1)
        end
    end
end

if test $removed_files -gt 0
    echo "Removed $removed_files GPY instant prompt cache files"
else
    echo "No GPY instant prompt cache files found"
end

echo ""
echo "Cache cleared. Your next prompt will fetch fresh data from the agent."
