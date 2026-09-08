#!/usr/bin/env fish
# Smoke test that theme export can be sourced and fish_prompt renders without errors

source (dirname (status --current-filename))/../support/setup_test_env.fish

function test_theme_prompt_render
    set -l tmp_dir (mktemp -d)
    set -l config_root $tmp_dir/config
    set -l theme_dir $config_root/gpy/themes
    mkdir -p $theme_dir

    # Copy default theme into temp config root using new schema
    cp config/themes/default.toml $theme_dir/test-theme.toml

    # Write config pointing to our temp theme
    mkdir -p $config_root/gpy
    printf "[ui]\ntheme = \"test-theme\"\n" >$config_root/gpy/config.toml

    # Run theme export and source it
    set -l theme_export $tmp_dir/theme-export.fish
    set -lx XDG_CONFIG_HOME $config_root
    set -l agent (get_agent_binary_path)
    $agent theme export --format fish >$theme_export

    source $theme_export

    # Initialize prompt and render once. No agent runs here, so the
    # agent-rendered segments (directory, git) are omitted; what the shell
    # itself must still produce is the theme's prompt character, and never
    # a JSON leak (#644). Directory/git content with a live agent is covered
    # by tests/fish/e2e_prompt_content.test.fish.
    set -gx GPY_AGENT_SOCKET_PATH $tmp_dir/no-agent.sock
    set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
    source fish/core/init.fish
    # Without this, `fish_prompt` is fish's own default prompt and the test
    # renders the wrong program entirely.
    source fish/functions/fish_prompt.fish
    set -l rendered (fish_prompt 2>/dev/null | string replace -ra '\e\[[0-9;]*m' '' | string collect)
    if not string match -q "*$__icon_prompt*" -- "$rendered"
        echo "❌ fish_prompt did not render the theme's prompt character '$__icon_prompt': $rendered"
        exit 1
    end
    if string match -q '*{"*' -- "$rendered"
        echo "❌ fish_prompt leaked JSON into the prompt: $rendered"
        exit 1
    end
    echo "✅ fish_prompt renders the prompt character with the exported theme"

    # Cleanup environment override
    set -e XDG_CONFIG_HOME
end

test_theme_prompt_render
