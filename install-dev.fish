#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# Contributor installer: build and install from this checkout. End users: install.sh.
# Usage: ./install-dev.fish [--fish-only] [--no-build] [--clean] [--help]

# Keep GPY out of the way while installing; a live prompt can hang the run.
set -gx GPY_AGENT_ENABLED 0
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Global so functions can read them.
set -g fish_only 0
set -g no_build 0
set -g clean_sync 0
set -g show_help 0

for arg in $argv
    switch $arg
        case --fish-only
            set fish_only 1
        case --no-build
            set no_build 1
        case --clean
            set clean_sync 1
        case --help -h
            set show_help 1
        case '*'
            echo "Error: Unknown option: $arg"
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
    exit 0
end

if test $fish_only -eq 1; and test $no_build -eq 0
    set no_build 1 # --fish-only implies --no-build
end

function _check_repo_root
    if not test -d gpy-agent
        echo "Error: gpy-agent directory not found"
        echo "   Run this script from the GPY repository root"
        return 1
    end
    return 0
end

function _check_rsync
    if not command -q rsync
        echo "Warning: rsync not found, falling back to cp"
        return 1
    end
    return 0
end

# Honors CARGO_TARGET_DIR, which cargo writes to instead of gpy-agent/target.
function _release_dir
    if set -q CARGO_TARGET_DIR; and test -n "$CARGO_TARGET_DIR"
        echo "$CARGO_TARGET_DIR/release"
    else
        echo gpy-agent/target/release
    end
end

function _build_agent
    echo "Building agent (release mode)..."
    cd gpy-agent
    if cargo build --release
        echo "Agent built successfully"
        cd ..
        return 0
    else
        echo "Error: Agent build failed"
        cd ..
        return 1
    end
end

function _stop_running_agent
    echo "Stopping any running agent..."

    if not command -q gpy-agent
        echo "No agent binary in PATH; nothing to stop"
        return 0
    end

    # `gpy-agent stop` already polls until the agent stops responding.
    set -l socket_path (gpy-agent status 2>/dev/null | string match -rg '^Socket Path: (.*)$')

    gpy-agent stop 2>/dev/null

    # Escalate only against the PID bound to our own socket, never by process
    # name: that would kill a dogfooding daemon or test agents on gpy-test-*.sock.
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
            echo "Warning: lsof not found; cannot safely stop the lingering agent (refusing to kill by process name)"
        end
    end

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

    echo "Agent stopped and cleaned up"
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
        echo "Cleared instant prompt cache"
    end
end

function _remove_conflicting_binaries --argument-names install_dir
    # Remove stale gpy/gpy-agent copies earlier on PATH that shadow the install
    # (e.g. ~/.cargo/bin). Only copies under $HOME are deleted: with a sandboxed
    # HOME, `sudo -E` or another account, PATH order says nothing about ownership.
    set -l target_dir (path resolve $install_dir)

    # Off PATH, nothing shadows the install; delete nothing.
    if not contains -- $target_dir (path resolve $PATH)
        for name in gpy gpy-agent
            set -l in_use (type -P $name 2>/dev/null)
            test -n "$in_use"; and echo "Note: $install_dir is not on PATH; $name resolves to $in_use instead (left in place)"
        end
        return 0
    end

    # Resolved so a symlinked HOME (/var -> /private/var) compares equal.
    # Empty HOME means nothing is deleted.
    set -l home_dir
    test -n "$HOME"; and set home_dir (path resolve $HOME)

    for name in gpy gpy-agent
        set -l reached_target 0
        for found in (type -aP $name 2>/dev/null)
            set -l found_dir (path resolve (path dirname $found))

            if test "$found_dir" = "$target_dir"
                set reached_target 1
                continue
            end
            test $reached_target -eq 1; and continue

            string match -q '*/shims/*' $found; and continue

            if not test -e $found
                continue
            end

            # Only delete our own binary; clap `--version` starts with its name.
            set -l found_version ("$found" --version 2>/dev/null)
            if not string match -q "$name *" -- $found_version
                echo "Warning: Found $name earlier on PATH but it doesn't look like ours, leaving it: $found"
                continue
            end

            # rm removes the directory entry, so the directory decides whether
            # it is under $HOME. Prefix compare: HOME may contain glob characters.
            set -l home_prefix "$home_dir/"
            if test -z "$home_dir"; or test (string sub -l (string length -- $home_prefix) -- "$found_dir/") != "$home_prefix"
                echo "Warning: $found shadows the install but is outside \$HOME, leaving it. If it is a stale copy, remove it: rm '$found'"
                continue
            end

            rm -f $found; and echo "Removed conflicting $name shadowing the install: $found"
        end
    end
end

function _install_agent_binary
    echo "Installing binaries..."

    set -l agent_binary (_release_dir)/gpy-agent
    set -l cli_binary (_release_dir)/gpy

    if not test -f $agent_binary
        echo "Error: Agent binary not found at $agent_binary"
        echo "   Build the agent first or use --fish-only"
        return 1
    end

    if not test -f $cli_binary
        echo "Error: CLI binary not found at $cli_binary"
        echo "   Build failed or incomplete"
        return 1
    end

    set -l install_dir "$HOME/.local/bin"
    mkdir -p $install_dir

    _stop_running_agent

    if command -q install
        install -m 755 $agent_binary $install_dir/gpy-agent
    else
        cp $agent_binary $install_dir/gpy-agent
        # Scoped so other extended attributes survive.
        if command -q xattr
            xattr -d com.apple.quarantine $install_dir/gpy-agent 2>/dev/null
        end
        chmod +x $install_dir/gpy-agent
    end

    if command -q install
        install -m 755 $cli_binary $install_dir/gpy
    else
        cp $cli_binary $install_dir/gpy
        if command -q xattr
            xattr -d com.apple.quarantine $install_dir/gpy 2>/dev/null
        end
        chmod +x $install_dir/gpy
    end

    echo "Installed to $install_dir/gpy"
    echo "Installed to $install_dir/gpy-agent"

    _remove_conflicting_binaries $install_dir

    return 0
end

function _install_fish_files
    echo "Installing Fish files..."

    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end

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

    set -l config_dirs config/themes
    set -l config_file config/config.toml

    set -l fish_files fish/core fish/segments fish/conf.d fish/functions

    for dir in $fish_files $config_dirs
        if not test -d $dir
            echo "Error: Source directory not found: $dir"
            return 1
        end
    end

    if not test -f $config_file
        echo "Error: Source file not found: $config_file"
        return 1
    end

    echo "   Installing configuration files to $gpy_config_dir..."
    # Never delete here, even under --clean: users' custom themes must survive.
    if _check_rsync
        rsync -a $config_dirs $gpy_config_dir/
    else
        cp -R $config_dirs $gpy_config_dir/
    end
    # Never overwrite an existing config.toml; ship the new default beside it.
    if not test -e $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml
    else if not cmp -s $config_file $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml.new
        echo "New default config available: $gpy_config_dir/config.toml.new"
        echo "   Your existing config.toml was left untouched; review the .new file for any changes."
    end

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

    # Fish autoloads completions/<command>.fish only, so the generated structural
    # completions and the gpy-dynamic.fish glue share completions/gpy.fish.
    set -l fish_completions_dir "$fish_config_dir/completions"
    mkdir -p $fish_completions_dir
    rm -f "$fish_completions_dir/gpy-dynamic.fish"

    set -l fish_completions_generated 0
    set -l dev_gpy_cli "$HOME/.local/bin/gpy"
    if test -x $dev_gpy_cli
        if $dev_gpy_cli completions fish >"$fish_completions_dir/gpy.fish"
            set fish_completions_generated 1
            echo "Generated structural completions: $fish_completions_dir/gpy.fish"
        else
            echo "Warning: Could not generate gpy completions (gpy completions fish failed)"
        end
    else
        echo "Warning: gpy CLI not installed; skipping structural shell completions"
    end

    if test -f fish/completions/gpy-dynamic.fish
        if test $fish_completions_generated -eq 1
            cat fish/completions/gpy-dynamic.fish >>"$fish_completions_dir/gpy.fish"
        else
            cat fish/completions/gpy-dynamic.fish >"$fish_completions_dir/gpy.fish"
        end
        echo "Installed dynamic completions glue into: $fish_completions_dir/gpy.fish"
    end

    if test -f fish/functions/fish_prompt.fish
        mkdir -p "$fish_config_dir/functions"
        set -l dest_prompt "$fish_config_dir/functions/fish_prompt.fish"
        set -l source_prompt "$gpy_fish_dir/functions/fish_prompt.fish"
        # Function scope: a `set -l` inside the block below is invisible after it.
        set -l skip_symlink 0

        # A foreign symlink (stow/yadm/chezmoi) is moved aside, never deleted. A
        # regular file is deleted as GPY's only when it carries both segment
        # dispatch lines every GPY prompt shipped with (the scripts/uninstall.*
        # rule); any other prompt, even one that mentions GPY, is backed up.
        if test -e $dest_prompt -o -L $dest_prompt
            if test -L $dest_prompt; and test (readlink $dest_prompt) = "$source_prompt"
                echo "fish_prompt symlink already correct"
                set skip_symlink 1
            else if test -f $dest_prompt -a ! -L $dest_prompt
                and grep -Eq '^[[:space:]]*for segment in \$__enabled_segments[[:space:]]*$' $dest_prompt
                and grep -Eq '^[[:space:]]*if functions -q segment_\{\$segment\}_render[[:space:]]*$' $dest_prompt
                rm -f $dest_prompt
            else
                set -l backup_file "$dest_prompt.backup."(date +%Y%m%d_%H%M%S)
                mv $dest_prompt $backup_file
                echo "Warning: Backed up existing prompt to: $backup_file"
            end
        end

        if test $skip_symlink -eq 0
            ln -sf "$source_prompt" "$dest_prompt"
            echo "Installed fish_prompt symlink"
        end
    end

    if test -d "$HOME/.config/gpy/core"
        echo ""
        echo "Warning: Old installation detected with Fish files in ~/.config/gpy/"
        echo "   GPY now separates configuration and implementation:"
        echo "   - Configuration: ~/.config/gpy/ (themes, config)"
        echo "   - Fish files: ~/.config/fish/gpy/ (core, segments, functions)"
        echo ""
        echo "   You can safely remove the old Fish files:"
        echo "   rm -rf ~/.config/gpy/{core,segments,conf.d,functions}"
        echo ""
    end

    echo "Configuration installed to $gpy_config_dir"
    echo "Fish files installed to $gpy_fish_dir"
    return 0
end

function _verify_installation
    echo ""
    echo "Verifying installation..."

    if command -q gpy-agent
        set -l agent_version (gpy-agent --version 2>/dev/null | string trim)
        if test -n "$agent_version"
            echo "Agent: $agent_version"
        else
            echo "Warning: Agent installed but version check failed"
        end
    else
        echo "Warning: Agent not in PATH (add ~/.local/bin to PATH)"
    end

    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end
    set -l gpy_fish_dir "$fish_config_dir/gpy"

    if test -d "$gpy_fish_dir/core"
        echo "Fish files installed"
    else
        echo "Error: Fish files not found"
        return 1
    end

    if command -q gpy-agent
        echo ""
        echo "Testing config hot-reload..."

        # The agent reopens GPY_DEBUG_LOG on every line, so the log lives in a
        # private temp dir and the long-lived daemon must not inherit it.
        set -l probe_dir (mktemp -d)
        set -l debug_log $probe_dir/agent.log
        env GPY_DEBUG_LOG=$debug_log gpy-agent start >/dev/null 2>&1

        set -l hot_reload_enabled 0
        for i in (seq 1 50)
            if test -f $debug_log; and grep -q "Config hot-reload enabled" $debug_log
                set hot_reload_enabled 1
                break
            end
            sleep 0.1
        end

        if test -f $debug_log
            if test $hot_reload_enabled -eq 1
                echo "Config hot-reload: ENABLED (1-second debounce)"
            else
                echo "Warning: Config hot-reload: UNKNOWN STATUS"
                echo "   Debug log: $debug_log (removed after the probe)"
            end

        else
            echo "Warning: Could not verify hot-reload (debug log not created)"
            echo "   Agent may not have started - check: gpy-agent status"
        end

        gpy-agent stop >/dev/null 2>&1
        rm -rf $probe_dir
        env -u GPY_DEBUG_LOG -u GPY_AGENT_ENABLED -u GPY_AGENT_SUPERVISOR_ENABLED gpy-agent start >/dev/null 2>&1
    end

    return 0
end

echo "GPY Development Installation"
echo ""

if not _check_repo_root
    exit 1
end

if test $no_build -eq 0; and test $fish_only -eq 0
    if not _build_agent
        exit 1
    end

    if not _install_agent_binary
        exit 1
    end
else
    echo "Skipping agent build"

    if test $fish_only -eq 0
        if not test -f (_release_dir)/gpy-agent
            echo "Error: Agent binary not found"
            echo "   Build the agent first or use --fish-only"
            exit 1
        end

        if not _install_agent_binary
            exit 1
        end
    end
end

if not _install_fish_files
    exit 1
end

if test $clean_sync -eq 1
    _clear_instant_prompt_cache
end

_verify_installation

echo ""
echo "Installation complete!"
echo ""

echo "Next steps:"
echo "  1. Restart Fish: exec fish"
echo "  2. Check prompt: type something and press Enter"
echo ""

if test $fish_only -eq 1
    echo "Tip: Use --fish-only for fast Fish-only updates during development"
else if test $clean_sync -eq 1
    echo "Tip: Used --clean for a pristine sync"
end

set -e fish_only
set -e no_build
set -e clean_sync
set -e show_help
