# SPDX-License-Identifier: GPL-3.0-or-later
#
# Dynamic-value completion glue for the `gpy` CLI: live theme, palette and
# segment names via `gpy __complete <kind>`. The installers append this file to
# the generated completions as `completions/gpy.fish`. Design notes:
# docs/dev/completions.md.
function __gpy_complete_cached --description 'Cache gpy __complete <kind> per session'
    set -l kind $argv[1]
    set -l cache_var __gpy_complete_cache_$kind
    if not set -q $cache_var
        set -l result (gpy __complete $kind 2>/dev/null)
        if test (count $result) -eq 0
            return 1
        end
        set -g $cache_var $result
    end
    for value in $$cache_var
        echo $value
    end
end

# True when exactly N words (command name included) precede the cursor, so a
# value is offered only in its own slot, as Bash (COMP_CWORD) and Zsh (CURRENT) do.
function __gpy_complete_at --argument-names position
    test (count (commandline -opc)) -eq $position
end

complete -c gpy -n '__fish_seen_subcommand_from theme; and __fish_seen_subcommand_from use; and __gpy_complete_at 3' -f -a '(__gpy_complete_cached theme)' -d Theme
complete -c gpy -n '__fish_seen_subcommand_from theme; and __fish_seen_subcommand_from validate; and __gpy_complete_at 3' -f -a '(__gpy_complete_cached theme)' -d Theme

complete -c gpy -n '__fish_seen_subcommand_from palette; and __fish_seen_subcommand_from use; and __gpy_complete_at 3' -f -a '(__gpy_complete_cached palette)' -d Palette
complete -c gpy -n '__fish_seen_subcommand_from palette; and __fish_seen_subcommand_from validate; and __gpy_complete_at 3' -f -a '(__gpy_complete_cached palette)' -d Palette

complete -c gpy -n '__fish_seen_subcommand_from enable; and __gpy_complete_at 2' -f -a '(__gpy_complete_cached segment)' -d Segment
complete -c gpy -n '__fish_seen_subcommand_from disable; and __gpy_complete_at 2' -f -a '(__gpy_complete_cached segment)' -d Segment
