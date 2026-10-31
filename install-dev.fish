#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# Contributor installer: build and install from this checkout. End users: install.sh.
# Usage: ./install-dev.fish [--fish-only] [--no-build] [--clean] [--allow-shadow] [--help]

# Keep GPY out of the way while installing; a live prompt can hang the run.
set -gx GPY_AGENT_ENABLED 0
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Global so functions can read them.
set -g fish_only 0
set -g no_build 0
set -g clean_sync 0
set -g show_help 0
set -g allow_shadow 0

for arg in $argv
    switch $arg
        case --fish-only
            set fish_only 1
        case --no-build
            set no_build 1
        case --clean
            set clean_sync 1
        case --allow-shadow
            set allow_shadow 1
        case --help -h
            set show_help 1
        case '*'
            echo "error: Unknown option: $arg"
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
    echo "  --clean         Remove the installed Fish tree before setup"
    echo "  --allow-shadow  Install even though a packaged gpy (e.g. Homebrew) is on PATH"
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

# This script installs into ~/.local/bin, which sits ahead of Homebrew, system
# and package-manager prefixes on PATH, and replaces ~/.config/fish/gpy that
# `gpy setup` owns. Doing that over a packaged gpy leaves a stale shadow
# once the package updates, so refuse unless asked. Other earlier copies are
# reported by `_warn_shadowing_binaries`.
function _refuse_to_shadow_package_install
    test $allow_shadow -eq 1; and return 0
    set -l install_dir (path resolve "$HOME/.local/bin")
    set -l home_dir (path resolve "$HOME")
    for found in (type -aP gpy 2>/dev/null)
        set -l found_dir (path resolve (path dirname $found))
        test "$found_dir" = "$install_dir"; and continue
        test (string sub -l (string length -- "$home_dir/") -- "$found_dir/") = "$home_dir/"; and continue
        echo "error: gpy is already installed at $found (not by this script)."
        echo "   This script would shadow it from $install_dir and replace the Fish files"
        echo "   that 'gpy setup' manages. To test unreleased code without touching it, use a"
        echo "   sandboxed HOME/XDG_* (see demo/setup.fish) or run the checkout's build directly."
        echo "   Pass --allow-shadow to install anyway."
        return 1
    end
    return 0
end

_refuse_to_shadow_package_install; or exit 1

function _check_repo_root
    if not test -d gpy-agent
        echo "error: gpy-agent directory not found"
        echo "   Run this script from the GPY repository root"
        return 1
    end
    return 0
end

# Resolved as gpy-agent/src/paths.rs does.
function _config_home
    if test -n "$XDG_CONFIG_HOME"
        echo "$XDG_CONFIG_HOME"
    else
        echo "$HOME/.config"
    end
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
        echo "ok: agent built"
        cd ..
        return 0
    else
        echo "error: Agent build failed"
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
            echo "warn: lsof not found; cannot safely stop the lingering agent (refusing to kill by process name)"
        end
    end

    if test -n "$socket_path"
        rm -f "$socket_path" 2>/dev/null
    end

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

# Reports gpy/gpy-agent copies earlier on PATH than the install dir; never touches them.
function _warn_shadowing_binaries --argument-names install_dir
    set -l target_dir (path resolve $install_dir)

    # Off PATH, nothing shadows the install.
    if not contains -- $target_dir (path resolve $PATH)
        for name in gpy gpy-agent
            set -l in_use (type -P $name 2>/dev/null)
            test -n "$in_use"; and echo "Note: $install_dir is not on PATH; $name resolves to $in_use instead (left in place)"
        end
        return 0
    end

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

            echo "warn: $found comes before $install_dir on PATH, so your shell runs it instead of this install. If it is stale, remove it: rm '$found'"
        end
    end
end

function _install_agent_binary
    echo "Installing binaries..."

    set -l install_dir "$HOME/.local/bin"
    set -l names gpy-agent gpy

    for name in $names
        if not test -f (_release_dir)/$name
            echo "error: $name binary not found at "(_release_dir)/$name
            echo "   Build first, or use --fish-only"
            return 1
        end
    end

    mkdir -p $install_dir

    _stop_running_agent

    for name in $names
        set -l built (_release_dir)/$name
        if command -q install
            install -m 755 $built $install_dir/$name
        else
            cp $built $install_dir/$name
            chmod +x $install_dir/$name
        end
        # Scoped so other extended attributes survive.
        if command -q xattr
            xattr -d com.apple.quarantine $install_dir/$name 2>/dev/null
        end
        echo "ok: installed $install_dir/$name"
    end

    _warn_shadowing_binaries $install_dir

    return 0
end

function _install_fish_files
    echo "Installing Fish files..."

    set -l config_home (_config_home)
    set -l fish_config_dir "$config_home/fish"
    set -l gpy_config_dir "$config_home/gpy"
    set -l gpy_fish_dir "$fish_config_dir/gpy"

    mkdir -p $gpy_config_dir
    set -l config_file config/config.example.toml

    if not test -f $config_file
        echo "error: Source file not found: $config_file"
        return 1
    end

    echo "   Installing configuration files to $gpy_config_dir..."
    # Never overwrite an existing config.toml; ship the new default beside it.
    if not test -e $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml
    else if not cmp -s $config_file $gpy_config_dir/config.toml
        cp $config_file $gpy_config_dir/config.toml.new
        echo "New default config available: $gpy_config_dir/config.toml.new"
        echo "   Your existing config.toml was left untouched; review the .new file for any changes."
    end

    if test $clean_sync -eq 1
        rm -rf "$fish_config_dir/gpy"
    end
    if not "$HOME/.local/bin/gpy" setup --shell fish --from . --non-interactive
        echo "error: gpy setup failed"
        return 1
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
            echo "warn: Agent installed but version check failed"
        end
    else
        echo "warn: Agent not in PATH (add ~/.local/bin to PATH)"
    end

    set -l gpy_fish_dir (_config_home)/fish/gpy

    if test -d "$gpy_fish_dir/core"
        echo "Fish files installed"
    else
        echo "error: Fish files not found"
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
                echo "warn: Config hot-reload: UNKNOWN STATUS"
                echo "   Debug log: $debug_log (removed after the probe)"
            end

        else
            echo "warn: Could not verify hot-reload (debug log not created)"
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
            echo "error: Agent binary not found"
            echo "   Build the agent first or use --fish-only"
            exit 1
        end

        if not _install_agent_binary
            exit 1
        end
    end
end

if not test -x "$HOME/.local/bin/gpy"
    echo "error: gpy is not installed; run ./install-dev.fish without --fish-only first"
    exit 1
end

if not _install_fish_files
    exit 1
end

if test $clean_sync -eq 1
    _clear_instant_prompt_cache
end

_verify_installation

echo ""
echo "ok: installation complete"
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
set -e allow_shadow
