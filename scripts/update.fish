#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later

function _check_for_updates
    echo "🔍 Checking for GPY updates..."

    # Get current version from Fisher config. fisher.json lives under fish/,
    # not at the repo root where this script expects to be invoked from (#324).
    set -l current_version (string match -r '"version":\s*"([^"]+)"' < fish/fisher.json | head -1 | sed 's/.*"version":\s*"\([^"]*\)".*/\1/')

    if test -z "$current_version"
        echo "❌ Could not determine current version from fisher.json"
        return 1
    end

    echo "📦 Current version: $current_version"

    # Check latest release from GitHub
    set -l latest_release (curl -s https://api.github.com/repos/jpease/gpy/releases/latest | string match -r '"tag_name":\s*"([^"]+)"' | head -1 | sed 's/.*"tag_name":\s*"\([^"]*\)".*/\1/')

    if test -z "$latest_release"
        echo "❌ Could not fetch latest release information"
        return 1
    end

    echo "🌟 Latest version: $latest_release"

    # Compare versions (simple string comparison - could be enhanced)
    if test "$current_version" = "$latest_release"
        echo "✅ GPY is already up to date ($current_version)"
        return 0
    else
        echo "📈 Update available: $current_version → $latest_release"
        return 2
    end
end

# Every install path (install.sh, install-oneline.sh, install-dev.fish) puts
# the binary in ~/.local/bin; a local `cargo install --path .` puts it in
# ~/.cargo/bin. Leaving both around lets PATH order silently decide which
# version runs (#324) -- copy Cargo's output into the canonical location so
# there's only ever one binary to find.
function _sync_cargo_binary_to_local_bin
    set -l cargo_bin "$HOME/.cargo/bin/gpy-agent"
    if not test -f "$cargo_bin"
        return 0
    end

    mkdir -p "$HOME/.local/bin"
    cp "$cargo_bin" "$HOME/.local/bin/gpy-agent"
    chmod +x "$HOME/.local/bin/gpy-agent"
    echo "✅ Synced Cargo-installed binary to ~/.local/bin/gpy-agent"
end

# gpy-agent is not published to crates.io (#502), so this used to run a forced
# `cargo install` of a crate that does not exist: on any machine with cargo the
# update aborted here and the Fish scripts were never refreshed. The binary
# ships in the release archive, so point at the installer that can actually
# replace it and let the rest of the update run.
# _verify_update still fails the run if no agent is present afterwards.
function _update_agent
    echo "🔨 Updating GPY agent..."

    _sync_cargo_binary_to_local_bin

    if not command -q gpy-agent
        echo "⚠️  GPY agent not found in PATH"
    end

    echo "📦 Agent binaries come from the release archive, not a package registry."
    echo "   Re-run the installer to pick up a new agent:"
    echo "   curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | sh"
    echo "   Or download a release: https://github.com/jpease/gpy/releases"

    return 0
end

function _update_fish_scripts
    echo "🐠 Updating Fish scripts..."

    # Source utilities to get access to helpers
    if test -f (dirname (status filename))/../fish/core/util.fish
        source (dirname (status filename))/../fish/core/util.fish
    else
        echo "❌ Core utilities not found"
        return 1
    end

    # Determine Fish config directory
    set -l fish_config_dir (__gpy_config_root)
    set -l prompt_dir "$fish_config_dir/gpy"
    set -l gpy_config_dir "$HOME/.config/gpy"
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set gpy_config_dir "$XDG_CONFIG_HOME/gpy"
    end

    # Backup current configuration
    if test -d "$prompt_dir"
        echo "💾 Backing up current configuration..."
        set -l backup_dir "$fish_config_dir/gpy.backup."(date +%Y%m%d_%H%M%S)
        cp -r "$prompt_dir" "$backup_dir"
        echo "   Backup created: $backup_dir"
    end

    # Update Fish scripts
    mkdir -p "$prompt_dir"
    mkdir -p "$gpy_config_dir"
    echo "📁 Updating Fish files in $prompt_dir..."

    # Copy updated fish implementation files
    for src in fish/core fish/segments fish/functions
        if not test -d "$src"
            echo "❌ Missing source directory: $src"
            return 1
        end
        set -l dest_dir "$prompt_dir/"(basename "$src")
        if command -q rsync
            rsync -a --delete "$src/" "$dest_dir/"
        else
            rm -rf "$dest_dir"
            cp -r "$src" "$prompt_dir/"
        end
    end

    # Copy updated themes to ~/.config/gpy/themes. Never delete: merge the
    # shipped theme files into place without removing anything not present
    # in the source, so a user's own custom theme file survives (#311).
    if not test -d config/themes
        echo "❌ Missing source directory: config/themes"
        return 1
    end
    mkdir -p "$gpy_config_dir/themes"
    if command -q rsync
        rsync -a config/themes/ "$gpy_config_dir/themes/"
    else
        cp -r config/themes/. "$gpy_config_dir/themes/"
    end

    # Copy the shipped config.toml only on first install. Once it exists,
    # never touch it in place -- write the shipped default beside it as
    # config.toml.new so the user can review/merge new options at their own
    # pace instead of losing customizations silently (#311).
    if test -f config/config.toml
        if not test -e "$gpy_config_dir/config.toml"
            cp config/config.toml "$gpy_config_dir/config.toml"
        else if not cmp -s config/config.toml "$gpy_config_dir/config.toml"
            cp config/config.toml "$gpy_config_dir/config.toml.new"
            echo "📝 New default config available: $gpy_config_dir/config.toml.new"
            echo "   Your existing config.toml was left untouched; review the .new file for any changes."
        end
    end

    # Update configuration files
    if test -f fish/conf.d/gpy_init.fish
        set -l conf_d_dir "$fish_config_dir/conf.d"
        mkdir -p "$conf_d_dir"
        cp fish/conf.d/gpy_init.fish "$conf_d_dir/"
        echo "✅ Configuration updated"
    end

    echo "✅ Fish scripts and configuration updated"
    return 0
end

function _restart_agent
    echo "🔄 Restarting GPY agent..."

    if not command -q gpy-agent
        echo "❌ Agent not found after update"
        return 1
    end

    # `gpy-agent start` is idempotent: a no-op if a matching version is
    # already running, and it self-heals a version mismatch by restarting
    # the old daemon onto the just-installed binary. Without this call, a
    # running old-version daemon keeps holding the socket indefinitely and
    # new shells silently speak a newer protocol to it (#307).
    if gpy-agent start >/dev/null 2>&1
        echo "✅ GPY agent restarted"
        return 0
    else
        echo "⚠️  Could not restart agent automatically (will start on next shell session)"
        return 0
    end
end

function _verify_update
    echo "🔍 Verifying update..."

    # Check agent version
    if command -q gpy-agent
        set -l agent_version (gpy-agent --version 2>/dev/null | string trim)
        if test -n "$agent_version"
            echo "✅ Agent version: $agent_version"
        else
            echo "⚠️  Could not determine agent version"
        end
    else
        echo "❌ Agent not found after update"
        return 1
    end

    # Check Fish functions
    if functions -q gpy_render_prompt
        echo "✅ Fish functions available"
    else
        echo "⚠️  Fish functions may not be loaded"
        echo "   Try restarting your Fish shell"
    end

    return 0
end

function update_gpy
    echo "=== GPY Update Script ==="
    echo ""

    # Check for updates
    _check_for_updates
    set -l update_status $status

    if test $update_status -eq 0
        # Already up to date
        return 0
    else if test $update_status -eq 2
        # Update available
        echo ""
    else
        # Error checking for updates
        echo "❌ Update check failed, continuing anyway..."
        echo ""
    end

    # Update agent
    if not _update_agent
        echo "❌ Agent update failed"
        return 1
    end

    # Update Fish scripts
    if not _update_fish_scripts
        echo "❌ Fish scripts update failed"
        return 1
    end

    # Restart agent so a previously running old-version daemon picks up the
    # new binary/protocol instead of continuing to serve stale shells.
    _restart_agent

    # Verify update
    if not _verify_update
        echo "⚠️  Update completed with warnings"
        return 1
    end

    echo ""
    echo "🎉 GPY updated successfully!"
    echo ""
    echo "📋 Next steps:"
    echo "   • Restart your Fish shell: exec fish"
    echo "   • Or reload config: source ~/.config/fish/config.fish"
    echo "   • Test with: gpy-agent --help"
    echo ""
    echo "📚 For issues, see: https://github.com/jpease/gpy/issues"

    return 0
end

update_gpy $argv
