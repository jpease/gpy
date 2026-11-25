# SPDX-License-Identifier: GPL-3.0-or-later
#
# One `gpy prompt fish` draws the prompt and, with --final-rendering (Fish
# 4.2+), the transient line left in scrollback. Without the binary, or when
# it fails, a bare symbol keeps the shell usable.

function fish_prompt
    set -l last_status $status
    set -l bin (__gpy_bin)
    if test -z "$bin"
        printf '❯ '
        return
    end
    set -l args prompt fish --status $last_status --pid $fish_pid --shell-version $FISH_VERSION
    set -q __gpy_instance; and set -a args --instance $__gpy_instance
    set -q __gpy_cmd_count; and set -a args --cmd $__gpy_cmd_count
    if set -q __gpy_duration; and string match -qr '\A[0-9]+\z' -- "$__gpy_duration"
        set -a args --duration-ms $__gpy_duration
    end
    # stderr stays the terminal: a tripped script segment says so there.
    set -l out (command $bin $args | string collect -N)
    if test "$pipestatus[1]" != 0; or not string match -q -- "*"\x1f"*" "$out"
        printf '❯ '
        return
    end
    # Each part may hold newlines: cut with `string replace`; `string
    # collect` drops the newline `string replace` adds.
    set -l prompt (string replace -r -- '\x1f[\s\S]*\z' '' "$out" | string collect)
    set -l tail (string replace -r -- '\A[^\x1f]*\x1f' '' "$out" | string collect)
    if contains -- --final-rendering $argv
        printf '%s' "$tail"
        return
    end
    # The binary leaves the transient line empty when nothing is to be
    # swapped in: Fish's own transient prompt is then off (only a value GPY
    # set is erased).
    if test "$__gpy_fish_transient_capable" = 1
        if test -n "$tail"
            if not set -q __gpy_owns_fish_transient_prompt
                set -g fish_transient_prompt 1
                set -g __gpy_owns_fish_transient_prompt 1
            end
        else if set -q __gpy_owns_fish_transient_prompt
            set -e -g fish_transient_prompt
            set -e -g __gpy_owns_fish_transient_prompt
        end
    end
    printf '%s' "$prompt"
end

# The `gpy` command: PATH first, then ~/.local/bin.
function __gpy_bin
    if command -q gpy
        command -s gpy
    else if test -n "$HOME"; and test -x "$HOME/.local/bin/gpy"
        echo $HOME/.local/bin/gpy
    else
        return 1
    end
end
