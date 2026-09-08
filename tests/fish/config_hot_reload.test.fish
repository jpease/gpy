#!/usr/bin/env fish
# Verifies that theme reload updates prompt-related vars without restarting.

set -l temp_root (mktemp -d)
set -l config_dir "$temp_root/gpy"
mkdir -p $config_dir
set -l config_path "$config_dir/config.toml"

# Initial config: language enabled with versions shown
printf "[language]\nenabled = true\nshow_versions = true\n\n[ui]\nshow_icons = false\ntheme = \"default\"\ndirectory.max_length = 80\nenabled_segments = [\"clock\",\"directory\",\"language\",\"git\"]\n" > $config_path

set -lx XDG_CONFIG_HOME $temp_root
set -lx XDG_CACHE_HOME "$temp_root/cache"
set -lx PATH $PWD/gpy-agent/target/debug $PATH
set -lx MISE_DISABLE 1

source fish/core/init.fish

function __gpy_read_instant_cache
    if test "$GPY_LANGUAGE_ENABLED" = 0
        echo ''
        return
    end
    if test "$GPY_LANGUAGE_SHOW_VERSIONS" = 0
        echo "\\e[0mlang"
    else
        echo "\\e[0mlang 1.23"
    end
end

set -gx _fake_project (mktemp -d)
cd $_fake_project

echo "Initial render uses versions"
set rendered (segment_language_render last)
if not string match -q '*1.23*' -- $rendered
    echo "❌ Expected version in initial render, got '$rendered'"
    exit 1
end

echo "Reload config with show_versions=false"
printf "[language]\nenabled = true\nshow_versions = false\n\n[ui]\nshow_icons = false\ntheme = \"default\"\ndirectory.max_length = 80\nenabled_segments = [\"clock\",\"directory\",\"language\",\"git\"]\n" > $config_path
# Reload theme by re-sourcing the theme export (simulates SIGUSR2 handler)
gpy-agent theme export --format fish 2>/dev/null | source

set rendered (segment_language_render last)
if string match -q '*1.23*' -- $rendered
    echo "❌ Expected version to be hidden after reload, got '$rendered'"
    exit 1
end

echo "Reload config with language disabled"
printf "[language]\nenabled = false\nshow_versions = false\n\n[ui]\nshow_icons = false\ntheme = \"default\"\ndirectory.max_length = 80\nenabled_segments = [\"clock\",\"directory\",\"language\",\"git\"]\n" > $config_path
# Reload theme by re-sourcing the theme export (simulates SIGUSR2 handler)
gpy-agent theme export --format fish 2>/dev/null | source

if segment_language_detect
    echo "❌ Language segment should not detect when disabled"
    exit 1
end

set rendered (segment_language_render last)
if test -n "$rendered"
    echo "❌ Expected no render output when disabled, got '$rendered'"
    exit 1
end

echo "✅ Config hot reload updates prompt state"
