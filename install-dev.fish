#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY Development Installation Script
# ===================================
# For active development use only. End users should use install.sh
#
# Usage:
#   ./install-dev.fish                # Full install (build agent + install fish files)
#   ./install-dev.fish --fish-only    # Skip agent build, only sync Fish files
#   ./install-dev.fish --no-build     # Use existing agent binary, skip build
#   ./install-dev.fish --clean        # Use rsync --delete for clean sync
#   ./install-dev.fish --bundle       # Create platform-specific bundle in repo/bin
#
# Flags:
#   --fish-only     Skip agent build, only update Fish files (fast iteration)
#   --no-build      Skip agent build, use existing binary in target/release/
#                   (or $CARGO_TARGET_DIR/release when that is set)
#   --clean         Clean sync with rsync --delete (removes stale files)
#   --bundle        Stage platform-specific bundle in repo/bin/
#   --help, -h      Show this help message

# Disable GPY during installation to prevent hanging
set -gx GPY_AGENT_ENABLED 0
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Parse arguments (use -g for global scope so functions can access them)
set -g fish_only 0
set -g no_build 0
set -g clean_sync 0
set -g create_bundle 0
set -g show_help 0

for arg in $argv
    switch $arg
        case --fish-only
            set fish_only 1
        case --no-build
            set no_build 1
        case --clean
            set clean_sync 1
        case --bundle
            set create_bundle 1
        case --help -h
            set show_help 1
        case '*'
            echo "❌ Unknown option: $arg"
            echo "   Run with --help for usage"
            exit 1
    end
end

if test $show_help -eq 1
    echo "GPY Development Installation Script"
    echo ""
    echo "Usage:"
    echo "  ./install-dev.fish [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --fish-only     Skip agent build, only update Fish files (fast)"
    echo "  --no-build      Skip building, use existing agent binary"
    echo "  --clean         Clean sync with rsync --delete"
    echo "  --bundle        Create platform-specific bundle in repo/bin/"
    echo "  --help, -h      Show this help"
    echo ""
    echo "Note:"
    echo "  The running agent is automatically stopped before installing"
    echo "  a new binary to prevent conflicts. You may need to restart"
    echo "  Fish shells or run 'gpy-agent start' after installation."
    echo ""
    echo "Examples:"
    echo "  ./install-dev.fish                    # Full install"
    echo "  ./install-dev.fish --fish-only        # Quick Fish update"
    echo "  ./install-dev.fish --clean --bundle   # Clean install with bundle"
    exit 0
end

# Conflict check
if test $fish_only -eq 1; and test $no_build -eq 0
    set no_build 1 # --fish-only implies --no-build
end

function _check_repo_root
    if not test -d gpy-agent
        echo "❌ gpy-agent directory not found"
        echo "   Run this script from the GPY repository root"
        return 1
    end
    return 0
end

function _check_rsync
    if not command -q rsync
        echo "⚠️  rsync not found, falling back to cp"
        return 1
    end
    return 0
end

# Where `cargo build --release` puts the binaries. Cargo honours
# CARGO_TARGET_DIR (a shared cache, a CI layout, the from-source install test
# in tests/bash/install_from_source_docs.test.bash); this script has to look
# where cargo wrote, not at a hardcoded gpy-agent/target (#635).
function _release_dir
    if set -q CARGO_TARGET_DIR; and test -n "$CARGO_TARGET_DIR"
        echo "$CARGO_TARGET_DIR/release"
    else
        echo gpy-agent/target/release
    end
end

function _build_agent
    echo "🔨 Building agent (release mode)..."
    cd gpy-agent
    if cargo build --release
        echo "✅ Agent built successfully"
        cd ..
        return 0
    else
        echo "❌ Agent build failed"
        cd ..
        return 1
    end
end

function _stop_running_agent
    echo "🛑 Stopping any running agent..."

    if not command -q gpy-agent
        echo "✅ No agent binary in PATH; nothing to stop"
        return 0
    end

    # Resolve this machine's own agent socket the same way `gpy-agent
    # status` reports it (the "Socket Path: ..." line), rather than assuming
    # a fixed location. `gpy-agent stop` already polls (bounded, #317,
    # SHUTDOWN_WAIT_MAX in gpy-agent/src/agent/lifecycle/mod.rs) until the
    # agent stops responding, so no fixed sleep is needed here either
    # (#615).
    set -l socket_path (gpy-agent status 2>/dev/null | string match -rg '^Socket Path: (.*)$')

    gpy-agent stop 2>/dev/null

    # Only escalate if the agent is still responding after a graceful stop.
    # Never match by process name: a bare `pkill -9 gpy-agent` used to kill
    # every gpy-agent on the host, including a contributor's live
    # dogfooding daemon and test agents bound to their own gpy-test-*.sock
    # (see scripts/cleanup-test-agents.sh) (#615). Only the PID actually
    # bound to this socket is targeted, TERM first then KILL after a
    # bounded poll.
    if test -n "$socket_path"; and gpy-agent status 2>/dev/null | string match -q '*Running and Responding*'
        if command -q lsof
            set -l socket_pid (lsof -t -- "$socket_path" 2>/dev/null)
            if test -n "$socket_pid"
                kill $socket_pid 2>/dev/null
                for i in (seq 1 20)
                    if not kill -0 $socket_pid 2>/dev/null
                        break
                    end
                    sleep 0.1
                end
                if kill -0 $socket_pid 2>/dev/null
                    kill -9 $socket_pid 2>/dev/null
                end
            end
        else
            echo "⚠️  lsof not found; cannot safely stop the lingering agent (refusing to kill by process name)"
        end
    end

    # Clean up stale sockets across all supported runtime locations
    if test -n "$socket_path"
        rm -f "$socket_path" 2>/dev/null
    end
    if set -q XDG_RUNTIME_DIR; and test -n "$XDG_RUNTIME_DIR"
        rm -f "$XDG_RUNTIME_DIR/gpy/gpy.sock" 2>/dev/null
    end
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"
        rm -f "$XDG_CACHE_HOME/gpy/gpy.sock" 2>/dev/null
    end
    rm -f "$HOME/.cache/gpy/gpy.sock" 2>/dev/null

    echo "✅ Agent stopped and cleaned up"
    return 0
end

function _clear_instant_prompt_cache
    set -l cache_dir
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"
        set cache_dir "$XDG_CACHE_HOME/gpy/instant-prompts"
    else
        set cache_dir "$HOME/.cache/gpy/instant-prompts"
    end

    if test -d "$cache_dir"
        rm -f "$cache_dir"/*.ansi 2>/dev/null
        echo "✅ Cleared instant prompt cache"
    end
end

function _remove_conflicting_binaries --argument-names install_dir
    # Remove stale gpy/gpy-agent builds that sit earlier on PATH than our
    # install dir and would therefore shadow the freshly installed binaries
    # (e.g. a leftover `cargo install` copy in ~/.cargo/bin). Tool-manager
    # shims (mise/asdf) are left alone: they fall through to PATH.
    #
    # Deletion is limited to copies under $HOME (#693). A shadowing copy
    # anywhere else (another user's home, /usr/local/bin, a Homebrew prefix)
    # is reported, not removed: with HOME pointed at a sandbox, `sudo -E` or a
    # second account, "earlier on PATH" says nothing about who owns the file.
    set -l target_dir (path resolve $install_dir)

    # If the install dir is not on PATH, nothing can shadow it, and every
    # copy on PATH is the one the user actually runs. Delete nothing; the
    # post-install verification already warns that the install is off PATH.
    if not contains -- $target_dir (path resolve $PATH)
        for name in gpy gpy-agent
            set -l in_use (type -P $name 2>/dev/null)
            test -n "$in_use"; and echo "ℹ️  $install_dir is not on PATH; $name resolves to $in_use instead (left in place)"
        end
        return 0
    end

    # Resolved so a symlinked HOME (/var -> /private/var on macOS) compares
    # equal to the resolved candidate directories below. Empty when HOME is
    # unset, in which case nothing is deleted.
    set -l home_dir
    test -n "$HOME"; and set home_dir (path resolve $HOME)

    for name in gpy gpy-agent
        set -l reached_target 0
        for found in (type -aP $name 2>/dev/null)
            set -l found_dir (path resolve (path dirname $found))

            # Stop at our own install: anything after it does not shadow us.
            if test "$found_dir" = "$target_dir"
                set reached_target 1
                continue
            end
            test $reached_target -eq 1; and continue

            # Leave tool-manager shims in place; they delegate to PATH.
            string match -q '*/shims/*' $found; and continue

            if not test -e $found
                continue
            end

            # Confirm this is actually our binary before deleting it: an
            # unrelated third-party tool earlier on PATH must not be removed
            # just because it happens to share the name (#324). Our clap
            # `--version` output starts with the binary name.
            set -l found_version ("$found" --version 2>/dev/null)
            if not string match -q "$name *" -- $found_version
                echo "⚠️  Found $name earlier on PATH but it doesn't look like ours, leaving it: $found"
                continue
            end

            # `rm` removes the directory entry, so the directory (not a
            # symlink's target) decides whether the file is under $HOME.
            # Plain prefix comparison: HOME may contain glob metacharacters.
            set -l home_prefix "$home_dir/"
            if test -z "$home_dir"; or test (string sub -l (string length -- $home_prefix) -- "$found_dir/") != "$home_prefix"
                echo "⚠️  $found shadows the install but is outside \$HOME, leaving it. If it is a stale copy, remove it: rm '$found'"
                continue
            end

            rm -f $found; and echo "🧹 Removed conflicting $name shadowing the install: $found"
        end
    end
end

function _install_agent_binary
    echo "📦 Installing binaries..."

    set -l agent_binary (_release_dir)/gpy-agent
    set -l cli_binary (_release_dir)/gpy

    if not test -f $agent_binary
        echo "❌ Agent binary not found at $agent_binary"
        echo "   Build the agent first or use --fish-only"
        return 1
    end

    if not test -f $cli_binary
        echo "❌ CLI binary not found at $cli_binary"
        echo "   Build failed or incomplete"
        return 1
    end

    set -l install_dir "$HOME/.local/bin"
    mkdir -p $install_dir

    # Stop any running agent before replacing binary
    _stop_running_agent

    # Install gpy-agent (internal daemon)
    if command -q install
        install -m 755 $agent_binary $install_dir/gpy-agent
    else
        cp $agent_binary $install_dir/gpy-agent
        # Scoped to com.apple.quarantine rather than `xattr -c`, which stripped
        # every extended attribute (#494). A locally built binary is not
        # quarantined in the first place; this is belt-and-braces for a binary
        # copied in from elsewhere.
        if command -q xattr
            xattr -d com.apple.quarantine $install_dir/gpy-agent 2>/dev/null
        end
        chmod +x $install_dir/gpy-agent
    end

    # Install gpy (user-facing CLI)
    if command -q install
        install -m 755 $cli_binary $install_dir/gpy
    else
        cp $cli_binary $install_dir/gpy
        if command -q xattr
            xattr -d com.apple.quarantine $install_dir/gpy 2>/dev/null
        end
        chmod +x $install_dir/gpy
    end

    echo "✅ Installed to $install_dir/gpy"
    echo "✅ Installed to $install_dir/gpy-agent"

    # Drop any older builds that would shadow what we just installed.
    _remove_conflicting_binaries $install_dir

    return 0
end

function _create_bundle
    echo "📦 Creating platform-specific bundle..."

    set -l agent_binary (_release_dir)/gpy-agent
    if not test -f $agent_binary
        echo "❌ Agent binary not found"
        return 1
    end

    # Determine platform and architecture
    set -l platform (uname -s | tr '[:upper:]' '[:lower:]')
    set -l arch (uname -m)

    switch $platform
        case darwin
            set platform macos
        case linux
            set platform linux
    end

    set -l bundle_name "gpy-agent-$platform-$arch"
    set -l bundle_dir bin

    mkdir -p $bundle_dir
    cp $agent_binary $bundle_dir/$bundle_name
    chmod +x $bundle_dir/$bundle_name

    echo "✅ Bundle created: $bundle_dir/$bundle_name"
    return 0
end

function _install_fish_files
    echo "🐠 Installing Fish files..."

    # Determine Fish config directory
    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end

    # Separate installation locations:
    # - Configuration files go to ~/.config/gpy/
    # - Fish implementation goes to ~/.config/fish/gpy/
    set -l gpy_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set gpy_config_dir "$XDG_CONFIG_HOME/gpy"
    else
        set gpy_config_dir "$HOME/.config/gpy"
    end
    set -l gpy_fish_dir "$fish_config_dir/gpy"
    set -l fish_conf_d_dir "$fish_config_dir/conf.d"

    mkdir -p $gpy_config_dir
    mkdir -p $gpy_fish_dir
    mkdir -p $fish_conf_d_dir

    # Configuration files/directories for ~/.config/gpy/
    set -l config_dirs config/themes
    set -l config_file config/config.toml

    # Fish implementation files for ~/.config/fish/gpy/
    set -l fish_files fish/core fish/segments fish/conf.d fish/functions

    # Check if all source directories exist
    for dir in $fish_files $config_dirs
        if not test -d $dir
            echo "❌ Source directory not found: $dir"
            return 1
        end
    end

    # Check if config file exists
    if not test -f $config_file
        echo "❌ Source file not found: $config_file"
        return 1
    end

    # Install configuration files to ~/.config/gpy/
    echo "   Installing configuration files to $gpy_config_dir..."
    # Copy directories. Never delete: even under --clean, merge shipped
    # theme files into place without removing anything not present in the
    # source, so a user's own custom theme file survives (#311).
    if _check_rsync
        rsync -a $config_dirs $gpy_config_dir/
    else
        cp -R $config_dirs $gpy_config_dir/
    end
    # Copy the shipped config.toml only on first install. Once it exists,
    # never touch it in place -- write the shipped default beside it as
    # config.toml.new so the user can review/merge new options at their own
    # pace instead of losing customizations silently (#311).
    if not test -e $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml
    else if not cmp -s $config_file $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml.new
        echo "📝 New default config available: $gpy_config_dir/config.toml.new"
        echo "   Your existing config.toml was left untouched; review the .new file for any changes."
    end

    # Install Fish implementation files to ~/.config/fish/gpy/
    echo "   Installing Fish files to $gpy_fish_dir..."
    if test $clean_sync -eq 1; and _check_rsync
        rsync -a --delete $fish_files $gpy_fish_dir/
    else if _check_rsync
        rsync -a $fish_files $gpy_fish_dir/
    else
        cp -R $fish_files $gpy_fish_dir/
    end

    if test -f fish/conf.d/gpy_init.fish
        cp fish/conf.d/gpy_init.fish "$fish_conf_d_dir/gpy_init.fish"
    end

    # Install Fish completions as ONE autoloadable file, `completions/gpy.fish`:
    # the STRUCTURAL completions (subcommand/flag names), regenerated from the
    # just-installed CLI on every run rather than checked in so they can never
    # drift from the binary on disk, followed by the small, checked-in
    # `completions/gpy-dynamic.fish` glue that supplies live
    # theme/palette/segment names via `gpy __complete <kind>` (#328, #329).
    # Fish autoloads `completions/<command>.fish` only for the command of that
    # name, so the glue must be appended to gpy.fish, not copied beside it
    # (#702). If the gpy CLI isn't on disk (e.g. `--fish-only`, which skips
    # the binary build/install) or generation fails, gpy.fish is the glue
    # alone; the prompt itself doesn't depend on shell completions.
    set -l fish_completions_dir "$fish_config_dir/completions"
    mkdir -p $fish_completions_dir
    # Older installs copied the glue as its own (never autoloaded) file.
    rm -f "$fish_completions_dir/gpy-dynamic.fish"

    set -l fish_completions_generated 0
    set -l dev_gpy_cli "$HOME/.local/bin/gpy"
    if test -x $dev_gpy_cli
        if $dev_gpy_cli completions fish >"$fish_completions_dir/gpy.fish"
            set fish_completions_generated 1
            echo "✅ Generated structural completions: $fish_completions_dir/gpy.fish"
        else
            echo "⚠️  Could not generate gpy completions (gpy completions fish failed)"
        end
    else
        echo "⚠️  gpy CLI not installed; skipping structural shell completions"
    end

    if test -f fish/completions/gpy-dynamic.fish
        if test $fish_completions_generated -eq 1
            cat fish/completions/gpy-dynamic.fish >>"$fish_completions_dir/gpy.fish"
        else
            cat fish/completions/gpy-dynamic.fish >"$fish_completions_dir/gpy.fish"
        end
        echo "✅ Installed dynamic completions glue into: $fish_completions_dir/gpy.fish"
    end

    # Install fish_prompt function as symlink (consistent with install.sh)
    if test -f fish/functions/fish_prompt.fish
        mkdir -p "$fish_config_dir/functions"
        set -l dest_prompt "$fish_config_dir/functions/fish_prompt.fish"
        set -l source_prompt "$gpy_fish_dir/functions/fish_prompt.fish"
        # Declared here (function scope), not inside the `if` block below: a
        # `set -l` inside an if/end block is scoped to that block in fish, so
        # setting it there never becomes visible to the check after `end`
        # (harmless today since re-linking an already-correct symlink is a
        # no-op, but the skip was never actually taking effect -- #324).
        set -l skip_symlink 0

        # Backup existing prompt if it's not already ours
        if test -e $dest_prompt
            # Check if it's already a symlink to our gpy version
            set -l is_gpy_symlink 0

            if test -L $dest_prompt
                set -l link_target (readlink $dest_prompt)
                if test "$link_target" = "$source_prompt"
                    set is_gpy_symlink 1
                end
            end

            if test $is_gpy_symlink -eq 0
                # Not our symlink - backup if it's a custom prompt
                if test -f $dest_prompt
                    if not grep -q -E "(GPY|__enabled_segments)" $dest_prompt 2>/dev/null
                        set -l backup_file "$dest_prompt.backup-"(date +%Y%m%d-%H%M%S)
                        mv $dest_prompt $backup_file
                        echo "⚠️  Backed up existing prompt to: $backup_file"
                    else
                        # It's an old GPY copy, remove it
                        rm -f $dest_prompt
                    end
                else if test -L $dest_prompt
                    # Symlink to somewhere else, remove it
                    rm -f $dest_prompt
                end
            else
                # Already correct symlink, no action needed
                echo "✅ fish_prompt symlink already correct"
                set skip_symlink 1
            end
        end

        # Create symlink if needed
        if test $skip_symlink -eq 0
            ln -sf "$source_prompt" "$dest_prompt"
            echo "✅ Installed fish_prompt symlink"
        end
    end

    # Check for old installation and provide migration notice
    if test -d "$HOME/.config/gpy/core"
        echo ""
        echo "⚠️  Old installation detected with Fish files in ~/.config/gpy/"
        echo "   GPY now separates configuration and implementation:"
        echo "   - Configuration: ~/.config/gpy/ (themes, config)"
        echo "   - Fish files: ~/.config/fish/gpy/ (core, segments, functions)"
        echo ""
        echo "   You can safely remove the old Fish files:"
        echo "   rm -rf ~/.config/gpy/{core,segments,conf.d,functions}"
        echo ""
    end

    echo "✅ Configuration installed to $gpy_config_dir"
    echo "✅ Fish files installed to $gpy_fish_dir"
    return 0
end

function _verify_installation
    echo ""
    echo "🔍 Verifying installation..."

    # Check agent binary
    if command -q gpy-agent
        set -l agent_version (gpy-agent --version 2>/dev/null | string trim)
        if test -n "$agent_version"
            echo "✅ Agent: $agent_version"
        else
            echo "⚠️  Agent installed but version check failed"
        end
    else
        echo "⚠️  Agent not in PATH (add ~/.local/bin to PATH)"
    end

    # Check Fish files
    # Determine Fish config directory
    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end
    set -l gpy_fish_dir "$fish_config_dir/gpy"

    if test -d "$gpy_fish_dir/core"
        echo "✅ Fish files installed"
    else
        echo "❌ Fish files not found"
        return 1
    end

    # Verify hot-reload capability (development builds only)
    if command -q gpy-agent
        echo ""
        echo "🔄 Testing config hot-reload..."

        # Probe with a private temp dir and a throwaway debug-logged agent.
        # The agent reopens GPY_DEBUG_LOG on every line, so the daemon that
        # outlives this script must never be the one carrying it (#743).
        set -l probe_dir (mktemp -d)
        set -l debug_log $probe_dir/agent.log
        env GPY_DEBUG_LOG=$debug_log gpy-agent start >/dev/null 2>&1

        # Poll for the hot-reload success line instead of a fixed sleep: the
        # agent logs it well under a second in practice, and a fixed 3s
        # sleep padded every dev install with dead time regardless (#615).
        set -l hot_reload_enabled 0
        for i in (seq 1 50)
            if test -f $debug_log; and grep -q "Config hot-reload enabled" $debug_log
                set hot_reload_enabled 1
                break
            end
            sleep 0.1
        end

        # Check for hot-reload success message
        if test -f $debug_log
            if test $hot_reload_enabled -eq 1
                echo "✅ Config hot-reload: ENABLED (1-second debounce)"
            else
                echo "⚠️  Config hot-reload: UNKNOWN STATUS"
                echo "   Debug log: $debug_log (removed after the probe)"
            end

        else
            echo "⚠️  Could not verify hot-reload (debug log not created)"
            echo "   Agent may not have started - check: gpy-agent status"
        end

        # Replace the debug-logged probe agent with a clean long-lived one.
        gpy-agent stop >/dev/null 2>&1
        rm -rf $probe_dir
        env -u GPY_DEBUG_LOG -u GPY_AGENT_ENABLED -u GPY_AGENT_SUPERVISOR_ENABLED gpy-agent start >/dev/null 2>&1
    end

    return 0
end

# Main installation flow
echo "🐠 GPY Development Installation"
echo "================================"
echo ""

# Check we're in the right place
if not _check_repo_root
    exit 1
end

# Build agent (unless skipped)
if test $no_build -eq 0; and test $fish_only -eq 0
    if not _build_agent
        exit 1
    end

    if not _install_agent_binary
        exit 1
    end

    # Create bundle if requested
    if test $create_bundle -eq 1
        _create_bundle
    end
else
    echo "⏭️  Skipping agent build"

    # If --fish-only, we don't even need the agent binary
    if test $fish_only -eq 0
        # But if --no-build, verify the binary exists
        if not test -f (_release_dir)/gpy-agent
            echo "❌ Agent binary not found"
            echo "   Build the agent first or use --fish-only"
            exit 1
        end

        if not _install_agent_binary
            exit 1
        end

        if test $create_bundle -eq 1
            _create_bundle
        end
    end
end

# Install Fish files
if not _install_fish_files
    exit 1
end

if test $clean_sync -eq 1
    _clear_instant_prompt_cache
end

# Verify installation
_verify_installation

echo ""
echo "🎉 Installation complete!"
echo ""

echo "Next steps:"
echo "  1. Restart Fish: exec fish"
echo "  2. Check prompt: type something and press Enter"
echo ""

if test $fish_only -eq 1
    echo "💡 Tip: Use --fish-only for fast Fish-only updates during development"
else if test $clean_sync -eq 1
    echo "💡 Tip: Used --clean for a pristine sync"
end

# Cleanup global variables
set -e fish_only
set -e no_build
set -e clean_sync
set -e create_bundle
set -e show_help
