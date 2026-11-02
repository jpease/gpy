#!/usr/bin/env fish
# Verifies that theme reload updates prompt-related vars without restarting.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l temp_root (mktemp -d)
set -l config_dir "$temp_root/gpy"
mkdir -p $config_dir
set -l config_path "$config_dir/config.toml"

# Initial config: language enabled with versions shown
printf "[language]\nshow_versions = true\n\n[ui]\nshow_icons = false\ntheme = \"default\"\ndirectory.max_length = 80\nenabled_segments = [\"clock\",\"directory\",\"language\",\"git\"]\n" >$config_path

set -lx XDG_CONFIG_HOME $temp_root
set -lx XDG_CACHE_HOME "$temp_root/cache"
set -lx PATH $PWD/gpy-agent/target/debug $PATH
set -lx MISE_DISABLE 1

source fish/core/init.fish

function __gpy_read_instant_cache
    echo "\\e[0mlang 1.23"
end

set -gx _fake_project (mktemp -d)
cd $_fake_project

echo "Initial render uses versions"
set rendered (segment_language_render last)
if not string match -q '*1.23*' -- $rendered
    fail "Expected version in initial render, got '$rendered'"
    exit 1
end

echo "Reload config with language unlisted"
printf "[language]\nshow_versions = false\n\n[ui]\nshow_icons = false\ntheme = \"default\"\ndirectory.max_length = 80\nenabled_segments = [\"clock\",\"directory\",\"git\"]\n" >$config_path
# Reload theme by re-sourcing the theme export (simulates the agent reload doorbell)
gpy-agent theme export --shell fish 2>/dev/null | source

# The reload must hand the new segment list to the shell: the prompt renders
# only the segments listed in __enabled_segments.
if contains language $__enabled_segments
    fail "Expected language to leave __enabled_segments after the reload, got '$__enabled_segments'"
    exit 1
end

test_finish "Config hot reload updates prompt state"
