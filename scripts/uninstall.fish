#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later

function uninstall_custom_prompt
    echo "🗑️  Uninstalling GPY..."

    # Determine Fish config directory. Inlined from fish/core/util.fish's
    # __gpy_config_root rather than sourcing that file: this script must run
    # standalone (e.g. after install.sh/install-oneline.sh, where there is no
    # local repo checkout to source from) (#310).
    set -l fish_config_dir "$HOME/.config/fish"
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    end
    set -l prompt_dir "$fish_config_dir/gpy"

    # XDG_CONFIG_HOME/XDG_CACHE_HOME-respecting, matching scripts/uninstall.sh
    # and the agent's own config/cache resolution (gpy-agent/src/paths.rs
    # config_root_for/cache_root_for) -- a hardcoded $HOME/.config or
    # $HOME/.cache here missed the real directories for anyone with either
    # variable set (#615).
    set -l config_home "$HOME/.config"
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set config_home "$XDG_CONFIG_HOME"
    end
    set -l cache_home "$HOME/.cache"
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"
        set cache_home "$XDG_CACHE_HOME"
    end

    set -l gpy_config_dir "$config_home/gpy"
    set -l gpy_cache_dir "$cache_home/gpy"
    set -l bin_dir "$HOME/.local/bin"
    set -l agent_binary "$bin_dir/gpy-agent"
    set -l cli_binary "$bin_dir/gpy"

    # The agent's runtime root (socket, agent.version, shells/, and the Fish
    # supervisor's PID file), resolved exactly as fish/core/ipc.fish's
    # __gpy_runtime_root and the agent's paths.rs do: XDG_RUNTIME_DIR first,
    # else the cache root (already removed below) (#642).
    set -l runtime_root "$gpy_cache_dir"
    if set -q XDG_RUNTIME_DIR; and test -n "$XDG_RUNTIME_DIR"; and string match -q '/*' -- "$XDG_RUNTIME_DIR"
        set runtime_root "$XDG_RUNTIME_DIR/gpy"
    end

    # Shell completions the installers write beside the user's own
    # (install.sh / install-oneline.sh: `gpy completions fish` plus the
    # dynamic-value glue), and the timestamped binary backups they leave in
    # ~/.local/bin on every upgrade (#642).
    set -l completions_dir "$fish_config_dir/completions"
    set -l completion_files "$completions_dir/gpy.fish" "$completions_dir/gpy-dynamic.fish"

    set -l config_file "$fish_config_dir/config.fish"

    echo "GPY will be uninstalled from the following locations:"
    echo "  - Fish files: $prompt_dir"
    echo "  - Agent binary: $agent_binary (and gpy-agent.backup.* copies)"
    echo "  - CLI binary: $cli_binary (and gpy.backup.* copies)"
    echo "  - Completions: $completions_dir/gpy.fish, gpy-dynamic.fish"
    echo "  - Configuration: $gpy_config_dir"
    echo "  - Cache: $gpy_cache_dir"
    echo "  - Runtime: $runtime_root"
    echo "  - fish_prompt.fish (will restore previous backup if available)"
    echo "  - conf.d/gpy_init.fish"
    echo "  - GPY block in $config_file (if exists)"
    echo ""
    echo "⚠️  This will also stop any running agent and supervisor processes"
    read -P "Press Enter to continue or Ctrl-C to cancel"

    # Remove files
    if test -d "$prompt_dir"
        rm -rf "$prompt_dir"
        echo "✅ Removed prompt files from $prompt_dir"
    else
        echo "🤔 No GPY directory found to remove."
    end

    set -l prompt_function_file "$fish_config_dir/functions/fish_prompt.fish"
    if test -L "$prompt_function_file"
        set -l linked_target (readlink "$prompt_function_file")
        if test "$linked_target" = "$prompt_dir/functions/fish_prompt.fish"
            rm "$prompt_function_file"
            echo "✅ Removed GPY fish_prompt symlink"
        else
            echo "🤔 fish_prompt.fish is a symlink to $linked_target; leaving untouched"
        end
    else if test -f "$prompt_function_file"
        if grep -q gpy "$prompt_function_file"
            rm "$prompt_function_file"
            echo "✅ Removed GPY fish_prompt implementation"
        end
    end

    # Restore previous fish_prompt backup if destination is absent (#667).
    # A non-GPY regular file, symlink (valid or dangling), directory, or other
    # existing destination must be preserved byte-for-byte, leaving all backups
    # untouched.
    if test -e "$prompt_function_file" -o -L "$prompt_function_file"
        echo "🤔 fish_prompt.fish already exists; preserving current prompt and backups"
    else
        # install.sh writes `fish_prompt.fish.gpy-backup.<stamp>`, install-oneline.sh
        # `fish_prompt.fish.backup.<stamp>`; both are ours to restore (#642).
        # Compare timestamp suffix descending; on equal stamps, lexicographical
        # order of the basename wins (.gpy-backup. over .backup.) (#670).
        set -l backup_candidates "$prompt_function_file".backup.* "$prompt_function_file".gpy-backup.*
        set -l eligible_backups
        for candidate in $backup_candidates
            if test -f "$candidate" -a ! -L "$candidate"
                set -l bname (basename "$candidate")
                set -l match (string match -r '^fish_prompt\.fish\.(backup|gpy-backup)\.([0-9]{8}_[0-9]{6})$' -- "$bname")
                if test (count $match) -eq 3
                    set -l stamp $match[3]
                    set eligible_backups $eligible_backups (printf "%s\t%s\t%s" "$stamp" "$bname" "$candidate")
                end
            end
        end

        if test (count $eligible_backups) -gt 0
            set -l winner_line (printf '%s\n' $eligible_backups | env LC_ALL=C sort | tail -n1)
            set -l latest_backup (printf '%s' "$winner_line" | cut -f3-)
            mv "$latest_backup" "$prompt_function_file"
            echo "♻️  Restored backup prompt from $latest_backup"
        end
    end

    # Remove conf.d file
    set -l conf_d_file "$fish_config_dir/conf.d/gpy_init.fish"
    if test -f "$conf_d_file"
        rm "$conf_d_file"
        echo "✅ Removed GPY initialization script from $conf_d_file"
    else
        echo "🤔 GPY conf.d initialization script not found. Skipping."
    end

    # Remove the GPY init block from config.fish (written by install.sh /
    # install-oneline.sh, or by manual installs following the same
    # convention). The block is delimited by "# >>> gpy-init >>>" / "# <<<
    # gpy-init <<<", with a blank separator line immediately preceding the
    # open marker for readability against any pre-existing content. To
    # restore the file byte-identically, the delete range starts at that
    # blank line (when present) through the close marker -- otherwise the
    # separator would be left behind as a stray trailing blank line (#310).
    if test -f "$config_file"
        set -l block_start "# >>> gpy-init >>>"
        set -l block_end "# <<< gpy-init <<<"
        if grep -qF -- "$block_start" "$config_file"
            set -l open_match (grep -nF -- "$block_start" "$config_file" | head -n1)
            set -l close_match (grep -nF -- "$block_end" "$config_file" | head -n1)
            set -l open_line (string split -m1 ':' -- $open_match)[1]
            set -l close_line (string split -m1 ':' -- $close_match)[1]

            if test -z "$close_line"
                echo "🤔 GPY block start found but no matching end marker in $config_file. Skipping removal to avoid corrupting the file."
            else
                set -l start_line $open_line
                if test "$open_line" -gt 1
                    set -l prev_line_num (math "$open_line - 1")
                    set -l prev_addr "$prev_line_num"p
                    set -l prev_line_content (sed -n "$prev_addr" "$config_file")
                    if test -z "$prev_line_content"
                        set start_line $prev_line_num
                    end
                end

                set -l sed_range "$start_line,$close_line"d
                sed -i.gpy1 -e "$sed_range" "$config_file"
                and rm "$config_file.gpy1"
                echo "✅ Removed GPY block from $config_file"
            end
        else
            echo "🤔 GPY block not found in $config_file. Skipping."
        end
    end

    # Stop agent and supervisor processes
    echo ""
    echo "🛑 Stopping GPY processes..."

    # Stop the Fish supervisor loop this install started: the one whose PID
    # fish/core/ipc.fish recorded under the runtime root, and only if that
    # PID still runs the loop. Never a host-wide `pgrep -f` by function name
    # -- that matched every user's and every test sandbox's supervisor on the
    # machine (#615, #642).
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
                echo "✅ Stopped supervisor process $supervisor_pid"
            else
                echo "🤔 PID $supervisor_pid in $supervisor_pidfile is not a GPY supervisor; leaving it alone"
            end
        end
        rm -f "$supervisor_pidfile" 2>/dev/null
    end

    # Then the agent itself, with the supervisor gone so nothing restarts it.
    # `gpy-agent stop` already waits (bounded) for the agent to go away.
    if test -x "$agent_binary"
        $agent_binary stop 2>/dev/null
        echo "✅ Sent stop command to agent"
    end

    # Remove agent binary
    if test -f "$agent_binary"
        rm "$agent_binary"
        echo "✅ Removed agent binary: $agent_binary"
    else
        echo "🤔 Agent binary not found at $agent_binary"
    end

    # Remove the CLI binary, and the timestamped backups both installers
    # leave beside the binaries on every upgrade (#642).
    if test -f "$cli_binary"
        rm "$cli_binary"
        echo "✅ Removed CLI binary: $cli_binary"
    else
        echo "🤔 CLI binary not found at $cli_binary"
    end
    set -l binary_backups $bin_dir/gpy-agent.backup.* $bin_dir/gpy.backup.*
    set -l removed_backups 0
    for backup in $binary_backups
        if test -e "$backup"
            rm -f "$backup"
            set removed_backups (math $removed_backups + 1)
        end
    end
    if test $removed_backups -gt 0
        echo "✅ Removed $removed_backups binary backup(s) from $bin_dir"
    end

    # Remove the completions the installers generated (#642).
    set -l removed_completions 0
    for completion in $completion_files
        if test -e "$completion"
            rm -f "$completion"
            set removed_completions (math $removed_completions + 1)
        end
    end
    if test $removed_completions -gt 0
        echo "✅ Removed $removed_completions completion file(s) from $completions_dir"
    end

    # Remove configuration directory
    if test -d "$gpy_config_dir"
        rm -rf "$gpy_config_dir"
        echo "✅ Removed configuration: $gpy_config_dir"
    else
        echo "🤔 Configuration directory not found"
    end

    # Remove cache directory
    if test -d "$gpy_cache_dir"
        rm -rf "$gpy_cache_dir"
        echo "✅ Removed cache: $gpy_cache_dir"
    else
        echo "🤔 Cache directory not found"
    end

    # Remove the runtime root (socket, agent.version, shells/) when it is
    # separate from the cache (#642).
    if test "$runtime_root" != "$gpy_cache_dir"; and test -d "$runtime_root"
        rm -rf "$runtime_root"
        echo "✅ Removed runtime: $runtime_root"
    end

    echo ""
    echo "🎉 Uninstall complete!"
    echo "🔄 Restart your shell to see changes."
    echo ""
    echo "📊 Removed:"
    echo "  • Fish prompt files and completions"
    echo "  • Agent and CLI binaries (and their backups)"
    echo "  • Configuration files"
    echo "  • Cache and runtime files"
    echo "  • Running processes"
end

uninstall_custom_prompt $argv
