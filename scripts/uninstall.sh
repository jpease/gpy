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

fish_config_dir="$config_home/fish"
fish_prompt_dir="$fish_config_dir/gpy"
fish_prompt_file="$fish_config_dir/functions/fish_prompt.fish"
fish_conf_d="$fish_config_dir/conf.d/gpy_init.fish"
fish_completions_dir="$fish_config_dir/completions"

clean_rc_file() {
    rc_file="$1"
    [ -f "$rc_file" ] || return 0

    block_start="# >>> gpy-init >>>"
    block_end="# <<< gpy-init <<<"

    if ! grep -qF -- "$block_start" "$rc_file" && ! grep -qF -- "$block_end" "$rc_file"; then
        echo "🤔 GPY block not found in $rc_file. Skipping."
        return 0
    fi

    tmp=$(mktemp "${TMPDIR:-/tmp}/gpy-uninstall.XXXXXX") || return 1
    diag=$(mktemp "${TMPDIR:-/tmp}/gpy-uninstall.XXXXXX") || { rm -f "$tmp"; return 1; }

    awk '
    BEGIN {
        start_marker = "# >>> gpy-init >>>"
        end_marker = "# <<< gpy-init <<<"
        n = 0
    }
    {
        lines[++n] = $0
    }
    END {
        i = 1
        has_missing_end = 0
        has_missing_start = 0
        removed_any = 0

        while (i <= n) {
            if (lines[i] == start_marker) {
                found_end = 0
                for (j = i + 1; j <= n; j++) {
                    if (lines[j] == end_marker) {
                        found_end = j
                        break
                    }
                    # A second start before any end means this start is
                    # orphaned; never pair it with the end of a later block.
                    if (lines[j] == start_marker) {
                        break
                    }
                }
                if (found_end > 0) {
                    del_start = i
                    if (i > 1 && lines[i - 1] ~ /^[[:space:]]*$/ && !marked_for_del[i - 1]) {
                        del_start = i - 1
                    }
                    for (k = del_start; k <= found_end; k++) {
                        marked_for_del[k] = 1
                    }
                    removed_any = 1
                    i = found_end + 1
                    continue
                } else {
                    has_missing_end = 1
                }
            } else if (lines[i] == end_marker) {
                has_missing_start = 1
            }
            i++
        }

        for (i = 1; i <= n; i++) {
            if (!marked_for_del[i]) {
                print lines[i]
            }
        }

        if (has_missing_end) {
            print "MISSING_END" > "/dev/stderr"
        }
        if (has_missing_start) {
            print "MISSING_START" > "/dev/stderr"
        }
        if (removed_any) {
            print "REMOVED" > "/dev/stderr"
        }
    }
    ' "$rc_file" > "$tmp" 2> "$diag"

    if grep -q "MISSING_END" "$diag"; then
        echo "🤔 GPY block start found without a matching end marker in $rc_file. Leaving that portion untouched; remove it by hand if it is stale."
    fi
    if grep -q "MISSING_START" "$diag"; then
        echo "🤔 GPY block end marker found without a start marker in $rc_file. Leaving it untouched; remove it by hand if it is stale."
    fi
    if grep -q "REMOVED" "$diag"; then
        # Write through the existing path rather than replacing it, so a
        # symlinked rc file (dotfile managers) keeps its link and the target
        # is edited, and the file keeps its mode. awk always ends output with
        # a newline; drop it again when the original had none.
        if [ -n "$(tail -c 1 "$rc_file")" ]; then
            printf '%s' "$(cat "$tmp")" > "$rc_file"
        else
            cat "$tmp" > "$rc_file"
        fi
        echo "✅ Removed GPY block from $rc_file"
    fi
    rm -f "$tmp" "$diag"
}

echo "🗑️  Uninstalling GPY..."
echo "GPY will be uninstalled globally for the current user from the following locations:"
echo "  - Shell integration files (including completions):"
echo "    - Bash: $gpy_config_dir/bash"
echo "    - Zsh: $gpy_config_dir/zsh"
echo "    - Fish: $fish_prompt_dir, $fish_conf_d, $fish_completions_dir/gpy*.fish"
echo "  - Agent binary: $agent_binary (and gpy-agent.backup.* copies)"
echo "  - CLI binary: $cli_binary (and gpy.backup.* copies)"
echo "  - Configuration: $gpy_config_dir"
echo "  - Cache: $gpy_cache_dir"
echo "  - Runtime: $runtime_root"
echo "  - fish_prompt.fish (will restore previous backup if destination becomes absent)"
echo "  - GPY init blocks from startup files (if present):"
echo "    - $HOME/.bashrc, $HOME/.bash_profile, $HOME/.zshrc, $fish_config_dir/config.fish"
echo ""
echo "⚠️  This will also stop any running agent and supervisor processes"
printf '%s' "Press Enter to continue or Ctrl-C to cancel: "
# Read and discard one line. `|| true` keeps a closed/EOF stdin (as used by
# non-interactive callers and tests) from tripping `set -e`.
read -r _gpy_uninstall_confirm || true

# Remove shell integration files
if [ -d "$gpy_config_dir/bash" ]; then
    rm -rf "$gpy_config_dir/bash"
    echo "✅ Removed bash integration files from $gpy_config_dir/bash"
fi
if [ -d "$gpy_config_dir/zsh" ]; then
    rm -rf "$gpy_config_dir/zsh"
    echo "✅ Removed zsh integration files from $gpy_config_dir/zsh"
fi

# Remove Fish integration files (#671)
if [ -d "$fish_prompt_dir" ]; then
    rm -rf "$fish_prompt_dir"
    echo "✅ Removed prompt files from $fish_prompt_dir"
fi

if [ -L "$fish_prompt_file" ]; then
    linked_target=$(readlink "$fish_prompt_file" 2>/dev/null || true)
    if [ "$linked_target" = "$fish_prompt_dir/functions/fish_prompt.fish" ]; then
        rm -f "$fish_prompt_file"
        echo "✅ Removed GPY fish_prompt symlink"
    else
        echo "🤔 fish_prompt.fish is a symlink to $linked_target; leaving untouched"
    fi
elif [ -f "$fish_prompt_file" ]; then
    if grep -q gpy "$fish_prompt_file" 2>/dev/null; then
        rm -f "$fish_prompt_file"
        echo "✅ Removed GPY fish_prompt implementation"
    fi
fi

# Restore previous fish_prompt backup if destination is absent (#667, #670, #671).
# A non-GPY regular file, symlink (valid or dangling), directory, or other
# existing destination must be preserved byte-for-byte, leaving all backups
# untouched.
if [ -e "$fish_prompt_file" ] || [ -L "$fish_prompt_file" ]; then
    echo "🤔 fish_prompt.fish already exists; preserving current prompt and backups"
else
    # Compare timestamp suffix descending; on equal stamps, lexicographical
    # order of the basename wins (.gpy-backup. over .backup.) (#670).
    TAB=$(printf '\t')
    eligible_backups=""
    for candidate in "$fish_prompt_file".backup.* "$fish_prompt_file".gpy-backup.*; do
        [ -f "$candidate" ] || continue
        [ ! -L "$candidate" ] || continue
        bname="${candidate##*/}"
        case "$bname" in
            fish_prompt.fish.backup.[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]_[0-9][0-9][0-9][0-9][0-9][0-9]|\
            fish_prompt.fish.gpy-backup.[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]_[0-9][0-9][0-9][0-9][0-9][0-9])
                stamp="${bname##*.}"
                entry="${stamp}${TAB}${bname}${TAB}${candidate}"
                if [ -z "$eligible_backups" ]; then
                    eligible_backups="$entry"
                else
                    eligible_backups="${eligible_backups}
${entry}"
                fi
                ;;
        esac
    done

    if [ -n "$eligible_backups" ]; then
        winner_line=$(printf "%s\n" "$eligible_backups" | LC_ALL=C sort | tail -n1)
        latest_backup=$(printf "%s" "$winner_line" | cut -f3-)
        mv "$latest_backup" "$fish_prompt_file"
        echo "♻️  Restored backup prompt from $latest_backup"
    fi
fi

if [ -f "$fish_conf_d" ]; then
    rm -f "$fish_conf_d"
    echo "✅ Removed GPY initialization script from $fish_conf_d"
fi

for completion in "$fish_completions_dir/gpy.fish" "$fish_completions_dir/gpy-dynamic.fish"; do
    if [ -f "$completion" ] || [ -L "$completion" ]; then
        rm -f "$completion"
        echo "✅ Removed completion file: $completion"
    fi
done

# Remove GPY init blocks from all supported startup files (#671)
for rc_file in "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.zshrc" "$fish_config_dir/config.fish"; do
    clean_rc_file "$rc_file"
done

# Stop agent and supervisor processes
echo ""
echo "🛑 Stopping GPY processes..."

# Stop the Fish supervisor loop if recorded PID matches (#671, mirroring uninstall.fish)
supervisor_pidfile="$runtime_root/supervisor.pid"
if [ -f "$supervisor_pidfile" ]; then
    supervisor_pid=$(cat "$supervisor_pidfile" 2>/dev/null | tr -d '[:space:]')
    case "$supervisor_pid" in
        ''|*[!0-9]*) ;;
        *)
            if kill -0 "$supervisor_pid" 2>/dev/null; then
                cmd=$(ps -o command= -p "$supervisor_pid" 2>/dev/null || true)
                case "$cmd" in
                    *__gpy_agent_supervisor_loop*)
                        kill "$supervisor_pid" 2>/dev/null || true
                        i=0
                        while [ $i -lt 20 ]; do
                            kill -0 "$supervisor_pid" 2>/dev/null || break
                            sleep 0.1 2>/dev/null || sleep 1
                            i=$((i + 1))
                        done
                        if kill -0 "$supervisor_pid" 2>/dev/null; then
                            kill -9 "$supervisor_pid" 2>/dev/null || true
                        fi
                        echo "✅ Stopped supervisor process $supervisor_pid"
                        ;;
                    *)
                        echo "🤔 PID $supervisor_pid in $supervisor_pidfile is not a GPY supervisor; leaving it alone"
                        ;;
                esac
            fi
            ;;
    esac
    rm -f "$supervisor_pidfile" 2>/dev/null
fi

if [ -x "$agent_binary" ]; then
    "$agent_binary" stop 2>/dev/null || true
    echo "✅ Sent stop command to agent"
fi
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
echo "  • Shell integration files and completions (bash, zsh, fish)"
echo "  • Agent and CLI binaries (and their backups)"
echo "  • Configuration files"
echo "  • Cache and runtime files"
echo "  • Running agent and supervisor processes"
