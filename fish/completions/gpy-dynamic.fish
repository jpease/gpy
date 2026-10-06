# SPDX-License-Identifier: GPL-3.0-or-later
#
# Dynamic-value completion glue for the `gpy` CLI.
#
# `gpy completions fish` (clap_complete) is regenerated at install time and
# covers the STRUCTURAL surface: subcommand and flag names, driven by the
# `__fish_gpy_needs_command` / `__fish_gpy_using_subcommand` predicates it
# defines. It has no way to know the live set of themes, palettes, or segments
# installed on this machine -- those come from the hidden `gpy __complete
# <kind>` command, which prints plain newline-delimited names (#328).
#
# This file supplies exactly those dynamic value lists. Unlike the generated
# output it is hand-authored and checked into the repo, so it ships unchanged
# regardless of `gpy`'s clap definitions.
#
# Installation (#702): fish autoloads `completions/<command>.fish` only for
# the command of that name, so this file is NEVER installed under its own
# name -- nothing would load it. The installers append it to the generated
# structural completions and write the result as a single
# `completions/gpy.fish` (the glue alone when the CLI is unavailable; its
# predicates need only fish's `__fish_seen_subcommand_from`). The `-n`
# predicates below therefore only narrow down to the specific value position
# (subcommand + action already seen); they don't redefine the subcommand/flag
# completions the generated part already provides.
#
# Per-session caching (#346): forking `gpy __complete <kind>` on every single
# TAB press is wasted work once the answer is already known for this shell
# session, so `__gpy_complete_cached` memoizes each kind's result in a
# session-global variable (`__gpy_complete_cache_<kind>`) the first time it's
# asked for and just replays it after that -- warm reads never fork. Only a
# NON-EMPTY result is cached: if `gpy __complete <kind>` fails or prints
# nothing (e.g. `gpy` missing from PATH), the cache variable is left unset so
# the next TAB retries instead of permanently caching the failure.
#
# Accepted staleness: a theme/palette/plugin installed mid-session will not
# show up in completions until a new shell is started (matches the general
# shell-session-cache tradeoff -- refresh by opening a new shell).
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

complete -c gpy -n '__fish_seen_subcommand_from theme; and __fish_seen_subcommand_from use' -f -a '(__gpy_complete_cached theme)' -d Theme
complete -c gpy -n '__fish_seen_subcommand_from theme; and __fish_seen_subcommand_from validate' -f -a '(__gpy_complete_cached theme)' -d Theme

complete -c gpy -n '__fish_seen_subcommand_from palette; and __fish_seen_subcommand_from use' -f -a '(__gpy_complete_cached palette)' -d Palette
complete -c gpy -n '__fish_seen_subcommand_from palette; and __fish_seen_subcommand_from validate' -f -a '(__gpy_complete_cached palette)' -d Palette

complete -c gpy -n '__fish_seen_subcommand_from enable' -f -a '(__gpy_complete_cached segment)' -d Segment
complete -c gpy -n '__fish_seen_subcommand_from disable' -f -a '(__gpy_complete_cached segment)' -d Segment
