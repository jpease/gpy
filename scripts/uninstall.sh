#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Remove the current user's whole GPY install (every shell, binaries, config,
# cache, runtime). Usage: sh scripts/uninstall.sh [--yes]
# Sources nothing, so a lone downloaded copy works.

set -e

usage() {
    echo "Usage: sh uninstall.sh [--yes]"
    echo "  -y, --yes   uninstall without asking (for scripts and non-interactive shells)"
}

error() { printf 'error: %s\n' "$1" >&2; }
warn() { printf 'warn: %s\n' "$1" >&2; }

assume_yes=0
for arg in "$@"; do
    case "$arg" in
        -y | --yes) assume_yes=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            error "unknown argument: $arg"
            usage >&2
            exit 2
            ;;
    esac
done

# Resolved as gpy-agent/src/paths.rs does.
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
cache_home="${XDG_CACHE_HOME:-$HOME/.cache}"

gpy_config_dir="$config_home/gpy"
gpy_cache_dir="$cache_home/gpy"
bin_dir="$HOME/.local/bin"
agent_binary="$bin_dir/gpy-agent"
cli_binary="$bin_dir/gpy"
share_dir="$HOME/.local/share/gpy"
bundled_plugins_dir="$gpy_config_dir/bundled-plugins"

# Absolute XDG_RUNTIME_DIR, else the cache root (as paths.rs).
runtime_root="$gpy_cache_dir"
case "${XDG_RUNTIME_DIR:-}" in
    /*) runtime_root="$XDG_RUNTIME_DIR/gpy" ;;
esac

fish_config_dir="$config_home/fish"
# ZDOTDIR may be unset here, so ~/.zshrc is cleaned too.
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

    # Note the created-file tag before the block is removed.
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
        warn "GPY block start found without a matching end marker in $rc_file. Leaving that portion untouched; remove it by hand if it is stale."
    fi
    if grep -q "MISSING_START" "$diag"; then
        warn "GPY block end marker found without a start marker in $rc_file. Leaving it untouched; remove it by hand if it is stale."
    fi
    if grep -q "REMOVED" "$diag"; then
        # Write through the path: keeps a symlinked rc file's link and its mode.
        # awk always adds a trailing newline; drop it if the original had none.
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
echo "  - Bundled plugins (username, hostname): $bundled_plugins_dir"
echo "  - Kept shell scripts and plugins: $share_dir"
echo "  - Configuration: $gpy_config_dir"
echo "  - Cache: $gpy_cache_dir"
echo "  - Runtime: $runtime_root"
echo "  - fish_prompt.fish (will restore previous backup if destination becomes absent)"
echo "  - GPY init blocks from startup files (if present):"
echo "    - $HOME/.bashrc, $HOME/.bash_profile, $HOME/.bash_login, $HOME/.profile, ${zdotdir_zshrc:+$zdotdir_zshrc, }$HOME/.zshrc, $fish_config_dir/config.fish"
echo ""
echo "This will also stop any running agent and supervisor processes"
if [ "$assume_yes" = 1 ]; then
    echo "Proceeding without confirmation (--yes)."
else
    printf '%s' "Press Enter to continue or Ctrl-C to cancel: "
    # EOF (closed or exhausted stdin) is not consent: nothing has been removed yet.
    if ! read -r _gpy_uninstall_confirm; then
        echo ""
        error "no confirmation received; nothing was removed. Re-run with --yes to uninstall without a prompt."
        exit 1
    fi
fi

remove_dir "$gpy_config_dir/bash" "Removed bash integration files from $gpy_config_dir/bash"
remove_dir "$gpy_config_dir/zsh" "Removed zsh integration files from $gpy_config_dir/zsh"
remove_dir "$bundled_plugins_dir" "Removed bundled plugins from $bundled_plugins_dir"

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
    # Early installers copied the GPY prompt in as a regular file. Every GPY
    # prompt ever shipped dispatches segments with these two lines; anything
    # else (including a prompt that merely mentions gpy) is the user's.
    # shellcheck disable=SC2016 # literal $ in the regexes
    if grep -Eq '^[[:space:]]*for segment in \$__enabled_segments[[:space:]]*$' "$fish_prompt_file" 2>/dev/null &&
        grep -Eq '^[[:space:]]*if functions -q segment_\{\$segment\}_render[[:space:]]*$' "$fish_prompt_file" 2>/dev/null; then
        rm -f "$fish_prompt_file"
        echo "Removed GPY fish_prompt implementation"
    else
        echo "fish_prompt.fish is not a GPY prompt; leaving untouched"
    fi
fi

# Restore the newest backup only when the destination is absent; otherwise
# leave it and the backups alone.
if [ -e "$fish_prompt_file" ] || [ -L "$fish_prompt_file" ]; then
    echo "fish_prompt.fish already exists; preserving current prompt and backups"
else
    # .gpy-backup.STAMP (install.sh), .backup.STAMP (install-oneline.sh,
    # install-dev.fish), legacy .backup-STAMP. Newest stamp wins; ties go to
    # the name sorting last. Symlinks into the GPY prompt dir never qualify.
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

# Kill the supervisor only if its recorded PID still runs the loop.
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

removed_backups=0
for backup in "$bin_dir"/gpy-agent.backup.* "$bin_dir"/gpy.backup.*; do
    [ -e "$backup" ] || continue
    rm -f "$backup"
    removed_backups=$((removed_backups + 1))
done
if [ "$removed_backups" -gt 0 ]; then
    echo "Removed $removed_backups binary backup(s) from $bin_dir"
fi
remove_dir "$share_dir" "Removed kept shell scripts and plugins: $share_dir"

remove_dir "$gpy_config_dir" "Removed configuration: $gpy_config_dir" "Configuration directory not found"
remove_dir "$gpy_cache_dir" "Removed cache: $gpy_cache_dir" "Cache directory not found"

if [ "$runtime_root" != "$gpy_cache_dir" ]; then
    remove_dir "$runtime_root" "Removed runtime: $runtime_root"
fi

echo ""
echo "ok: GPY uninstalled"
echo "Restart your shell to see changes."
echo ""
echo "Removed:"
echo "  - Shell integration files and completions (bash, zsh, fish)"
echo "  - Agent and CLI binaries (and their backups)"
echo "  - Configuration files"
echo "  - Cache and runtime files"
echo "  - Running agent and supervisor processes"
