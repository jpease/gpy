#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY Uninstaller for Zsh/Bash
#
# Usage: sh scripts/uninstall.sh
#        GPY_SHELL=zsh sh scripts/uninstall.sh
#
# Mirrors scripts/uninstall.fish for the zsh/bash integration installed by
# install-oneline.sh. Standalone: does not source or otherwise depend on a
# local repo checkout, so it works for anyone who installed via
# install-oneline.sh and has no clone of the gpy repo lying around (#310).

set -e

# Detect which shell's integration to remove, mirroring install-oneline.sh's
# precedence: an explicit GPY_SHELL override, else $SHELL's basename, else
# fall back to bash.
current_shell="${SHELL##*/}"
if [ -n "$GPY_SHELL" ]; then
    current_shell="$GPY_SHELL"
fi
case "$current_shell" in
    zsh | bash) ;;
    *) current_shell="bash" ;;
esac

# XDG_CONFIG_HOME/XDG_CACHE_HOME-respecting, matching both
# install-oneline.sh's bash/zsh config dir and the agent's own config/cache
# resolution (gpy-agent/src/paths.rs config_root_for/cache_root_for) -- a
# hardcoded $HOME/.config or $HOME/.cache here missed the real directories
# for anyone with either variable set (#615).
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
cache_home="${XDG_CACHE_HOME:-$HOME/.cache}"

shell_config_dir="$config_home/gpy/$current_shell"
gpy_config_dir="$config_home/gpy"
gpy_cache_dir="$cache_home/gpy"
bin_dir="$HOME/.local/bin"
agent_binary="$bin_dir/gpy-agent"
cli_binary="$bin_dir/gpy"

# The agent's runtime root (socket, agent.version, shells/), resolved as the
# shells and the agent's paths.rs do: an absolute XDG_RUNTIME_DIR first, else
# the cache root (removed below anyway) (#642).
runtime_root="$gpy_cache_dir"
case "${XDG_RUNTIME_DIR:-}" in
    /*) runtime_root="$XDG_RUNTIME_DIR/gpy" ;;
esac

if [ "$current_shell" = "zsh" ]; then
    rc_file="$HOME/.zshrc"
else
    # Same precedence install-oneline.sh uses when configuring Bash: prefer
    # .bashrc, fall back to .bash_profile.
    if [ -f "$HOME/.bashrc" ]; then
        rc_file="$HOME/.bashrc"
    else
        rc_file="$HOME/.bash_profile"
    fi
fi

echo "🗑️  Uninstalling GPY..."
echo "GPY will be uninstalled from the following locations:"
echo "  - $current_shell integration files (including completions): $shell_config_dir"
echo "  - Agent binary: $agent_binary (and gpy-agent.backup.* copies)"
echo "  - CLI binary: $cli_binary (and gpy.backup.* copies)"
echo "  - Configuration: $gpy_config_dir"
echo "  - Cache: $gpy_cache_dir"
echo "  - Runtime: $runtime_root"
echo "  - GPY block in $rc_file (if exists)"
echo ""
echo "⚠️  This will also stop any running agent process"
printf '%s' "Press Enter to continue or Ctrl-C to cancel: "
# Read and discard one line. `|| true` keeps a closed/EOF stdin (as used by
# non-interactive callers and tests) from tripping `set -e`.
read -r _gpy_uninstall_confirm || true

# Remove shell integration files
if [ -d "$shell_config_dir" ]; then
    rm -rf "$shell_config_dir"
    echo "✅ Removed $current_shell integration files from $shell_config_dir"
else
    echo "🤔 No $current_shell integration directory found to remove."
fi

# Remove the GPY init block from the rc file (written by install-oneline.sh).
# The block is delimited by "# >>> gpy-init >>>" / "# <<< gpy-init <<<", with
# a blank separator line immediately preceding the open marker for
# readability against any pre-existing content. To restore the file
# byte-identically, the delete range starts at that blank line (when
# present) through the close marker -- otherwise the separator would be left
# behind as a stray trailing blank line (#310).
if [ -f "$rc_file" ]; then
    block_start="# >>> gpy-init >>>"
    block_end="# <<< gpy-init <<<"
    if grep -qF -- "$block_start" "$rc_file"; then
        open_line=$(grep -nF -- "$block_start" "$rc_file" | head -n1 | cut -d: -f1)
        close_line=$(grep -nF -- "$block_end" "$rc_file" | head -n1 | cut -d: -f1)

        if [ -z "$close_line" ]; then
            echo "🤔 GPY block start found but no matching end marker in $rc_file. Skipping removal to avoid corrupting the file."
        else
            start_line=$open_line
            if [ "$open_line" -gt 1 ]; then
                prev_line_num=$((open_line - 1))
                prev_line_content=$(sed -n "${prev_line_num}p" "$rc_file")
                if [ -z "$prev_line_content" ]; then
                    start_line=$prev_line_num
                fi
            fi

            sed -i.gpy1 -e "${start_line},${close_line}d" "$rc_file"
            rm -f "$rc_file.gpy1"
            echo "✅ Removed GPY block from $rc_file"
        fi
    else
        echo "🤔 GPY block not found in $rc_file. Skipping."
    fi
fi

# Stop agent process
echo ""
echo "🛑 Stopping GPY processes..."

if [ -x "$agent_binary" ]; then
    "$agent_binary" stop 2>/dev/null || true
    echo "✅ Sent stop command to agent"
fi

# `gpy-agent stop` already waits (bounded) for the agent to go away.

# Remove agent binary
if [ -f "$agent_binary" ]; then
    rm -f "$agent_binary"
    echo "✅ Removed agent binary: $agent_binary"
else
    echo "🤔 Agent binary not found at $agent_binary"
fi

# Remove the CLI binary, and the timestamped backups install-oneline.sh
# leaves beside the binaries on every upgrade (#642).
if [ -f "$cli_binary" ]; then
    rm -f "$cli_binary"
    echo "✅ Removed CLI binary: $cli_binary"
else
    echo "🤔 CLI binary not found at $cli_binary"
fi
removed_backups=0
for backup in "$bin_dir"/gpy-agent.backup.* "$bin_dir"/gpy.backup.*; do
    [ -e "$backup" ] || continue
    rm -f "$backup"
    removed_backups=$((removed_backups + 1))
done
if [ "$removed_backups" -gt 0 ]; then
    echo "✅ Removed $removed_backups binary backup(s) from $bin_dir"
fi

# Remove configuration directory
if [ -d "$gpy_config_dir" ]; then
    rm -rf "$gpy_config_dir"
    echo "✅ Removed configuration: $gpy_config_dir"
else
    echo "🤔 Configuration directory not found"
fi

# Remove cache directory
if [ -d "$gpy_cache_dir" ]; then
    rm -rf "$gpy_cache_dir"
    echo "✅ Removed cache: $gpy_cache_dir"
else
    echo "🤔 Cache directory not found"
fi

# Remove the runtime root (socket, agent.version, shells/) when it is
# separate from the cache (#642).
if [ "$runtime_root" != "$gpy_cache_dir" ] && [ -d "$runtime_root" ]; then
    rm -rf "$runtime_root"
    echo "✅ Removed runtime: $runtime_root"
fi

echo ""
echo "🎉 Uninstall complete!"
echo "🔄 Restart your shell to see changes."
echo ""
echo "📊 Removed:"
echo "  • $current_shell integration files and completions"
echo "  • Agent and CLI binaries (and their backups)"
echo "  • Configuration files"
echo "  • Cache and runtime files"
echo "  • Running agent process"
