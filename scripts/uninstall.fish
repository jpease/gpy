#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY Uninstaller (fish)
#
# Usage: fish scripts/uninstall.fish
#
# Removes the current user's whole GPY install, exactly the set of files
# scripts/uninstall.sh removes. It sources nothing, so a lone downloaded copy
# works without a repo checkout; for the same reason it cannot hand off to
# uninstall.sh, which may not be beside it.

# __gpy_remove_dir DIR REMOVED_MSG [MISSING_MSG]
function __gpy_remove_dir --argument-names dir removed_msg missing_msg
    if test -d "$dir"
        rm -rf "$dir"
        echo "$removed_msg"
    else if test -n "$missing_msg"
        echo "$missing_msg"
    end
end

# __gpy_remove_file FILE REMOVED_MSG [MISSING_MSG]
function __gpy_remove_file --argument-names file removed_msg missing_msg
    if test -f "$file"
        rm "$file"
        echo "$removed_msg"
    else if test -n "$missing_msg"
        echo "$missing_msg"
    end
end

# Remove each existing path in $argv and print how many were removed.
function __gpy_remove_existing
    set -l removed 0
    for target in $argv
        if test -e "$target"
            rm -f "$target"
            set removed (math $removed + 1)
        end
    end
    echo $removed
end

function __gpy_clean_rc_file --argument-names rc_file
    test -f "$rc_file"; or return 0

    set -l block_start "# >>> gpy-init >>>"
    set -l block_end "# <<< gpy-init <<<"

    if not grep -qF -- "$block_start" "$rc_file"; and not grep -qF -- "$block_end" "$rc_file"
        echo "GPY block not found in $rc_file. Skipping."
        return 0
    end

    # The installer tags a block in a file it created; remember that before
    # the block is removed.
    set -l created_by_gpy 0
    if grep -qF -- "(gpy-created-file)" "$rc_file"
        set created_by_gpy 1
    end

    set -l tmp_root /tmp
    set -q TMPDIR; and test -n "$TMPDIR"; and set tmp_root $TMPDIR
    set -l tmp (mktemp "$tmp_root/gpy-uninstall.XXXXXX"); or return 1
    set -l diag (mktemp "$tmp_root/gpy-uninstall.XXXXXX"); or begin
        rm -f "$tmp"
        return 1
    end

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
    ' "$rc_file" >"$tmp" 2>"$diag"

    if grep -q MISSING_END "$diag"
        echo "GPY block start found without a matching end marker in $rc_file. Leaving that portion untouched; remove it by hand if it is stale."
    end
    if grep -q MISSING_START "$diag"
        echo "GPY block end marker found without a start marker in $rc_file. Leaving it untouched; remove it by hand if it is stale."
    end
    if grep -q REMOVED "$diag"
        # Write through the existing path so a symlinked rc file keeps its
        # link (the target is edited) and the file keeps its mode. awk always
        # ends output with a newline; drop it when the original had none.
        set -l last_byte (tail -c 1 "$rc_file")
        if test -n "$last_byte"
            printf '%s' (cat "$tmp" | string collect) >"$rc_file"
        else
            cat "$tmp" >"$rc_file"
        end
        echo "Removed GPY block from $rc_file"
        if test $created_by_gpy = 1; and not grep -q '[^[:space:]]' "$rc_file"
            rm -f "$rc_file"
            echo "Removed $rc_file (created by GPY, now empty)"
        end
    end
    rm -f "$tmp" "$diag"
end

function uninstall_custom_prompt
    echo "Uninstalling GPY..."

    # Config/cache roots resolve as fish/core/util.fish's __gpy_config_root
    # and the agent (gpy-agent/src/paths.rs config_root_for/cache_root_for)
    # resolve them. Inlined, not sourced, so the script runs standalone.
    set -l config_home "$HOME/.config"
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set config_home "$XDG_CONFIG_HOME"
    end
    set -l cache_home "$HOME/.cache"
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"
        set cache_home "$XDG_CACHE_HOME"
    end

    set -l fish_config_dir "$config_home/fish"
    set -l prompt_dir "$fish_config_dir/gpy"
    set -l gpy_config_dir "$config_home/gpy"
    set -l gpy_cache_dir "$cache_home/gpy"
    set -l bin_dir "$HOME/.local/bin"
    set -l agent_binary "$bin_dir/gpy-agent"
    set -l cli_binary "$bin_dir/gpy"

    # The agent's runtime root (socket, agent.version, shells/, and the fish
    # supervisor's PID file), resolved exactly as fish/core/ipc.fish's
    # __gpy_runtime_root and the agent's paths.rs do: an absolute
    # XDG_RUNTIME_DIR first, else the cache root (removed below anyway).
    set -l runtime_root "$gpy_cache_dir"
    if set -q XDG_RUNTIME_DIR; and test -n "$XDG_RUNTIME_DIR"; and string match -q '/*' -- "$XDG_RUNTIME_DIR"
        set runtime_root "$XDG_RUNTIME_DIR/gpy"
    end

    # Completions the installers write beside the user's own: gpy.fish
    # (`gpy completions fish` plus the dynamic-value glue) and the standalone
    # gpy-dynamic.fish that older installs may still have.
    set -l completions_dir "$fish_config_dir/completions"
    set -l completion_files "$completions_dir/gpy.fish" "$completions_dir/gpy-dynamic.fish"

    set -l config_file "$fish_config_dir/config.fish"
    set -l conf_d_file "$fish_config_dir/conf.d/gpy_init.fish"
    # zsh reads ${ZDOTDIR:-$HOME}/.zshrc; ZDOTDIR may be unset in the shell
    # that runs the uninstaller, so ~/.zshrc is cleaned too.
    set -l zshrc_files "$HOME/.zshrc"
    if set -q ZDOTDIR; and test -n "$ZDOTDIR"; and test "$ZDOTDIR/.zshrc" != "$HOME/.zshrc"
        set zshrc_files "$ZDOTDIR/.zshrc" "$HOME/.zshrc"
    end
    set -l zshrc_files_text (string join ", " -- $zshrc_files)

    echo "GPY will be uninstalled globally for the current user from the following locations:"
    echo "  - Shell integration files (including completions):"
    echo "    - Fish: $prompt_dir, $conf_d_file, $completions_dir/gpy*.fish"
    echo "    - Bash/Zsh: $gpy_config_dir/bash, $gpy_config_dir/zsh"
    echo "  - Agent binary: $agent_binary (and gpy-agent.backup.* copies)"
    echo "  - CLI binary: $cli_binary (and gpy.backup.* copies)"
    echo "  - Configuration: $gpy_config_dir"
    echo "  - Cache: $gpy_cache_dir"
    echo "  - Runtime: $runtime_root"
    echo "  - fish_prompt.fish (will restore previous backup if destination becomes absent)"
    echo "  - GPY init blocks from startup files (if present):"
    echo "    - $HOME/.bashrc, $HOME/.bash_profile, $HOME/.bash_login, $HOME/.profile, $zshrc_files_text, $config_file"
    echo ""
    echo "This will also stop any running agent and supervisor processes"
    read -P "Press Enter to continue or Ctrl-C to cancel"

    __gpy_remove_dir "$prompt_dir" "Removed prompt files from $prompt_dir" "No GPY directory found to remove."

    set -l prompt_function_file "$fish_config_dir/functions/fish_prompt.fish"
    if test -L "$prompt_function_file"
        set -l linked_target (readlink "$prompt_function_file")
        if test "$linked_target" = "$prompt_dir/functions/fish_prompt.fish"
            rm "$prompt_function_file"
            echo "Removed GPY fish_prompt symlink"
        else
            echo "fish_prompt.fish is a symlink to $linked_target; leaving untouched"
        end
    else if test -f "$prompt_function_file"
        if grep -q gpy "$prompt_function_file"
            rm "$prompt_function_file"
            echo "Removed GPY fish_prompt implementation"
        end
    end

    # Restore the newest fish_prompt backup only when the destination is
    # absent. Any existing destination (a non-GPY regular file, a symlink
    # valid or dangling, a directory) is preserved byte-for-byte and the
    # backups are left alone.
    if test -e "$prompt_function_file" -o -L "$prompt_function_file"
        echo "fish_prompt.fish already exists; preserving current prompt and backups"
    else
        # Every GPY backup name is ours to restore: install.sh writes
        # `.gpy-backup.YYYYMMDD_HHMMSS`, install-oneline.sh and install-dev.fish
        # `.backup.YYYYMMDD_HHMMSS`, and older install-dev.fish runs left
        # `.backup-YYYYMMDD-HHMMSS` (stamp normalised to `_` for comparison).
        # The newest stamp wins; on equal stamps the basename that sorts last
        # wins (.gpy-backup. over .backup. over .backup-).
        # A backup may be a symlink (a stow/yadm prompt moved aside), valid or
        # dangling, but never one pointing back into the GPY prompt dir.
        set -l backup_candidates "$prompt_function_file".backup.* "$prompt_function_file".gpy-backup.* "$prompt_function_file".backup-*
        set -l eligible_backups
        for candidate in $backup_candidates
            if test -f "$candidate" -o -L "$candidate"
                if test -L "$candidate"
                    set -l candidate_target (readlink "$candidate")
                    if string match -q -- "$prompt_dir/*" "$candidate_target"
                        continue
                    end
                end
                set -l bname (basename "$candidate")
                set -l match (string match -r '^fish_prompt\.fish\.(backup|gpy-backup)\.([0-9]{8}_[0-9]{6})$' -- "$bname")
                if test (count $match) -eq 3
                    set eligible_backups $eligible_backups (printf "%s\t%s\t%s" "$match[3]" "$bname" "$candidate")
                else
                    set match (string match -r '^fish_prompt\.fish\.backup-([0-9]{8})-([0-9]{6})$' -- "$bname")
                    if test (count $match) -eq 3
                        set eligible_backups $eligible_backups (printf "%s_%s\t%s\t%s" "$match[2]" "$match[3]" "$bname" "$candidate")
                    end
                end
            end
        end

        if test (count $eligible_backups) -gt 0
            set -l winner_line (printf '%s\n' $eligible_backups | env LC_ALL=C sort | tail -n1)
            set -l latest_backup (printf '%s' "$winner_line" | cut -f3-)
            mv "$latest_backup" "$prompt_function_file"
            echo "Restored backup prompt from $latest_backup"
        end
    end

    __gpy_remove_file "$conf_d_file" "Removed GPY initialization script from $conf_d_file" "GPY conf.d initialization script not found. Skipping."

    for rc_file in "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile" $zshrc_files "$config_file"
        __gpy_clean_rc_file "$rc_file"
    end

    echo ""
    echo "Stopping GPY processes..."

    # Stop the fish supervisor loop this install started: the one whose PID
    # fish/core/ipc.fish recorded under the runtime root, and only if that
    # PID still runs the loop. Never a host-wide `pgrep -f` by function name:
    # that would match every user's and every test sandbox's supervisor.
    set -l supervisor_pidfile "$runtime_root/supervisor.pid"
    if test -f "$supervisor_pidfile"
        set -l supervisor_pid (string trim -- (cat "$supervisor_pidfile" 2>/dev/null))
        if string match -qr '^[0-9]+$' -- "$supervisor_pid"; and kill -0 $supervisor_pid 2>/dev/null
            if ps -o command= -p $supervisor_pid 2>/dev/null | string match -q '*__gpy_agent_supervisor_loop*'
                kill $supervisor_pid 2>/dev/null
                for i in (seq 1 20)
                    kill -0 $supervisor_pid 2>/dev/null; or break
                    sleep 0.1
                end
                kill -0 $supervisor_pid 2>/dev/null; and kill -9 $supervisor_pid 2>/dev/null
                echo "Stopped supervisor process $supervisor_pid"
            else
                echo "PID $supervisor_pid in $supervisor_pidfile is not a GPY supervisor; leaving it alone"
            end
        end
        rm -f "$supervisor_pidfile" 2>/dev/null
    end

    # Then the agent itself, with the supervisor gone so nothing restarts it.
    # `gpy-agent stop` already waits (bounded) for the agent to go away.
    if test -x "$agent_binary"
        $agent_binary stop 2>/dev/null
        echo "Sent stop command to agent"
    end

    __gpy_remove_file "$agent_binary" "Removed agent binary: $agent_binary" "Agent binary not found at $agent_binary"
    __gpy_remove_file "$cli_binary" "Removed CLI binary: $cli_binary" "CLI binary not found at $cli_binary"

    # Both installers leave timestamped backups beside the binaries on every
    # upgrade. Globbed through `set`, which tolerates no matches.
    set -l binary_backups $bin_dir/gpy-agent.backup.* $bin_dir/gpy.backup.*
    set -l removed_backups (__gpy_remove_existing $binary_backups)
    if test $removed_backups -gt 0
        echo "Removed $removed_backups binary backup(s) from $bin_dir"
    end

    set -l removed_completions (__gpy_remove_existing $completion_files)
    if test $removed_completions -gt 0
        echo "Removed $removed_completions completion file(s) from $completions_dir"
    end

    __gpy_remove_dir "$gpy_config_dir" "Removed configuration: $gpy_config_dir" "Configuration directory not found"
    __gpy_remove_dir "$gpy_cache_dir" "Removed cache: $gpy_cache_dir" "Cache directory not found"

    # The runtime root holds the socket, agent.version and shells/; it needs
    # its own removal only when it is separate from the cache.
    if test "$runtime_root" != "$gpy_cache_dir"
        __gpy_remove_dir "$runtime_root" "Removed runtime: $runtime_root"
    end

    echo ""
    echo "Uninstall complete!"
    echo "Restart your shell to see changes."
    echo ""
    echo "Removed:"
    echo "  • Shell integration files and completions (bash, zsh, fish)"
    echo "  • Agent and CLI binaries (and their backups)"
    echo "  • Configuration files"
    echo "  • Cache and runtime files"
    echo "  • Running processes"
end

uninstall_custom_prompt $argv
