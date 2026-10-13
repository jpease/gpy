#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY Uninstaller (POSIX sh)
#
# Usage: sh scripts/uninstall.sh
#
# Removes the current user's whole GPY install: every shell's integration
# (bash, zsh and fish), the binaries and their upgrade backups, config, cache
# and runtime files. GPY_SHELL, which install-oneline.sh passes, is accepted
# but not needed. The script sources nothing, so a lone downloaded copy works
# without a repo checkout. scripts/uninstall.fish is the fish entry point and
# must remove the same set of files.

set -e

# Config/cache roots resolve as install-oneline.sh and the agent
# (gpy-agent/src/paths.rs config_root_for/cache_root_for) resolve them.
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
cache_home="${XDG_CACHE_HOME:-$HOME/.cache}"

gpy_config_dir="$config_home/gpy"
gpy_cache_dir="$cache_home/gpy"
bin_dir="$HOME/.local/bin"
agent_binary="$bin_dir/gpy-agent"
cli_binary="$bin_dir/gpy"

# The agent's runtime root (socket, agent.version, shells/), resolved as the
# shells and the agent's paths.rs do: an absolute XDG_RUNTIME_DIR first, else
# the cache root (removed below anyway).
runtime_root="$gpy_cache_dir"
case "${XDG_RUNTIME_DIR:-}" in
    /*) runtime_root="$XDG_RUNTIME_DIR/gpy" ;;
esac

fish_config_dir="$config_home/fish"
# zsh reads ${ZDOTDIR:-$HOME}/.zshrc; ZDOTDIR may be unset in the shell that
# runs the uninstaller, so ~/.zshrc is cleaned too.
zdotdir_zshrc=""
if [ -n "${ZDOTDIR:-}" ] && [ "$ZDOTDIR/.zshrc" != "$HOME/.zshrc" ]; then
    zdotdir_zshrc="$ZDOTDIR/.zshrc"
fi
fish_prompt_dir="$fish_config_dir/gpy"
fish_prompt_file="$fish_config_dir/functions/fish_prompt.fish"
fish_conf_d="$fish_config_dir/conf.d/gpy_init.fish"
fish_completions_dir="$fish_config_dir/completions"

# remove_dir DIR REMOVED_MSG [MISSING_MSG]
remove_dir() {
    if [ -d "$1" ]; then
        rm -rf "$1"
        echo "$2"
    elif [ -n "${3:-}" ]; then
        echo "$3"
    fi
}

# remove_file FILE REMOVED_MSG [MISSING_MSG]
remove_file() {
    if [ -f "$1" ]; then
        rm -f "$1"
        echo "$2"
    elif [ -n "${3:-}" ]; then
        echo "$3"
    fi
}

clean_rc_file() {
    rc_file="$1"
    [ -f "$rc_file" ] || return 0

    block_start="# >>> gpy-init >>>"
    block_end="# <<< gpy-init <<<"

    if ! grep -qF -- "$block_start" "$rc_file" && ! grep -qF -- "$block_end" "$rc_file"; then
        echo "GPY block not found in $rc_file. Skipping."
        return 0
    fi

    # The installer tags a block in a file it created; remember that before
    # the block is removed.
    created_by_gpy=0
    if grep -qF -- "(gpy-created-file)" "$rc_file"; then
        created_by_gpy=1
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
        echo "GPY block start found without a matching end marker in $rc_file. Leaving that portion untouched; remove it by hand if it is stale."
    fi
    if grep -q "MISSING_START" "$diag"; then
        echo "GPY block end marker found without a start marker in $rc_file. Leaving it untouched; remove it by hand if it is stale."
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
        echo "Removed GPY block from $rc_file"
        if [ "$created_by_gpy" = 1 ] && ! grep -q '[^[:space:]]' "$rc_file"; then
            rm -f "$rc_file"
            echo "Removed $rc_file (created by GPY, now empty)"
        fi
    fi
    rm -f "$tmp" "$diag"
}

echo "Uninstalling GPY..."
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
echo "    - $HOME/.bashrc, $HOME/.bash_profile, $HOME/.bash_login, $HOME/.profile, ${zdotdir_zshrc:+$zdotdir_zshrc, }$HOME/.zshrc, $fish_config_dir/config.fish"
echo ""
echo "This will also stop any running agent and supervisor processes"
printf '%s' "Press Enter to continue or Ctrl-C to cancel: "
# Read and discard one line. `|| true` keeps a closed/EOF stdin (as used by
# non-interactive callers and tests) from tripping `set -e`.
read -r _gpy_uninstall_confirm || true

remove_dir "$gpy_config_dir/bash" "Removed bash integration files from $gpy_config_dir/bash"
remove_dir "$gpy_config_dir/zsh" "Removed zsh integration files from $gpy_config_dir/zsh"

# Fish integration is removed too, so either uninstaller leaves no shell's
# files behind.
remove_dir "$fish_prompt_dir" "Removed prompt files from $fish_prompt_dir"

if [ -L "$fish_prompt_file" ]; then
    linked_target=$(readlink "$fish_prompt_file" 2>/dev/null || true)
    if [ "$linked_target" = "$fish_prompt_dir/functions/fish_prompt.fish" ]; then
        rm -f "$fish_prompt_file"
        echo "Removed GPY fish_prompt symlink"
    else
        echo "fish_prompt.fish is a symlink to $linked_target; leaving untouched"
    fi
elif [ -f "$fish_prompt_file" ]; then
    if grep -q gpy "$fish_prompt_file" 2>/dev/null; then
        rm -f "$fish_prompt_file"
        echo "Removed GPY fish_prompt implementation"
    fi
fi

# Restore the newest fish_prompt backup only when the destination is absent.
# Any existing destination (a non-GPY regular file, a symlink valid or
# dangling, a directory) is preserved byte-for-byte and the backups are left
# alone.
if [ -e "$fish_prompt_file" ] || [ -L "$fish_prompt_file" ]; then
    echo "fish_prompt.fish already exists; preserving current prompt and backups"
else
    # Every GPY backup name is ours to restore: install.sh writes
    # `.gpy-backup.YYYYMMDD_HHMMSS`, install-oneline.sh and install-dev.fish
    # `.backup.YYYYMMDD_HHMMSS`, and older install-dev.fish runs left
    # `.backup-YYYYMMDD-HHMMSS` (stamp normalised to `_` for comparison).
    # The newest stamp wins; on equal stamps the basename that sorts last
    # wins (.gpy-backup. over .backup. over .backup-).
    # A backup may itself be a symlink (a stow/yadm prompt moved aside), valid
    # or dangling, but never one pointing back into the GPY prompt dir.
    TAB=$(printf '\t')
    eligible_backups=""
    for candidate in "$fish_prompt_file".backup.* "$fish_prompt_file".gpy-backup.* "$fish_prompt_file".backup-*; do
        [ -f "$candidate" ] || [ -L "$candidate" ] || continue
        if [ -L "$candidate" ]; then
            case "$(readlink "$candidate" 2>/dev/null || true)" in
                "$fish_prompt_dir"/*) continue ;;
            esac
        fi
        bname="${candidate##*/}"
        case "$bname" in
            fish_prompt.fish.backup.[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]_[0-9][0-9][0-9][0-9][0-9][0-9]|\
            fish_prompt.fish.gpy-backup.[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]_[0-9][0-9][0-9][0-9][0-9][0-9])
                stamp="${bname##*.}"
                ;;
            fish_prompt.fish.backup-[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]-[0-9][0-9][0-9][0-9][0-9][0-9])
                stamp="${bname##*backup-}"
                stamp="${stamp%-*}_${stamp#*-}"
                ;;
            *) continue ;;
        esac
        entry="${stamp}${TAB}${bname}${TAB}${candidate}"
        if [ -z "$eligible_backups" ]; then
            eligible_backups="$entry"
        else
            eligible_backups="${eligible_backups}
${entry}"
        fi
    done

    if [ -n "$eligible_backups" ]; then
        winner_line=$(printf "%s\n" "$eligible_backups" | LC_ALL=C sort | tail -n1)
        latest_backup=$(printf "%s" "$winner_line" | cut -f3-)
        mv "$latest_backup" "$fish_prompt_file"
        echo "Restored backup prompt from $latest_backup"
    fi
fi

remove_file "$fish_conf_d" "Removed GPY initialization script from $fish_conf_d"

for completion in "$fish_completions_dir/gpy.fish" "$fish_completions_dir/gpy-dynamic.fish"; do
    if [ -f "$completion" ] || [ -L "$completion" ]; then
        rm -f "$completion"
        echo "Removed completion file: $completion"
    fi
done

for rc_file in "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile" ${zdotdir_zshrc:+"$zdotdir_zshrc"} "$HOME/.zshrc" "$fish_config_dir/config.fish"; do
    clean_rc_file "$rc_file"
done

echo ""
echo "Stopping GPY processes..."

# Stop the fish supervisor loop whose PID fish/core/ipc.fish recorded under
# the runtime root, and only if that PID still runs the loop.
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
                        echo "Stopped supervisor process $supervisor_pid"
                        ;;
                    *)
                        echo "PID $supervisor_pid in $supervisor_pidfile is not a GPY supervisor; leaving it alone"
                        ;;
                esac
            fi
            ;;
    esac
    rm -f "$supervisor_pidfile" 2>/dev/null
fi

if [ -x "$agent_binary" ]; then
    "$agent_binary" stop 2>/dev/null || true
    echo "Sent stop command to agent"
fi
remove_file "$agent_binary" "Removed agent binary: $agent_binary" "Agent binary not found at $agent_binary"
remove_file "$cli_binary" "Removed CLI binary: $cli_binary" "CLI binary not found at $cli_binary"

# install-oneline.sh leaves timestamped backups beside the binaries on every
# upgrade.
removed_backups=0
for backup in "$bin_dir"/gpy-agent.backup.* "$bin_dir"/gpy.backup.*; do
    [ -e "$backup" ] || continue
    rm -f "$backup"
    removed_backups=$((removed_backups + 1))
done
if [ "$removed_backups" -gt 0 ]; then
    echo "Removed $removed_backups binary backup(s) from $bin_dir"
fi

remove_dir "$gpy_config_dir" "Removed configuration: $gpy_config_dir" "Configuration directory not found"
remove_dir "$gpy_cache_dir" "Removed cache: $gpy_cache_dir" "Cache directory not found"

# The runtime root holds the socket, agent.version and shells/; it needs its
# own removal only when it is separate from the cache.
if [ "$runtime_root" != "$gpy_cache_dir" ]; then
    remove_dir "$runtime_root" "Removed runtime: $runtime_root"
fi

echo ""
echo "Uninstall complete!"
echo "Restart your shell to see changes."
echo ""
echo "Removed:"
echo "  • Shell integration files and completions (bash, zsh, fish)"
echo "  • Agent and CLI binaries (and their backups)"
echo "  • Configuration files"
echo "  • Cache and runtime files"
echo "  • Running agent and supervisor processes"
