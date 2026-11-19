#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Theme Refresh After an Offline Config Edit
#
# A config.toml change made while the agent is down is never seen by the
# agent's file watcher, so no `.reload` is produced for it when it happens.
# Every shell must still draw the new theme:
#
#   A. A new shell whose first prompt comes after the offline edit (its
#      `gpy prompt fish` starts the agent) draws the new theme at once.
#   B. An idle interactive shell open across agent stop -> config edit ->
#      agent start repaints with the new theme with no keystroke (the
#      restarted agent rings it).
#
# The `default` theme frames segments in powerline chevrons (U+E0B0..U+E0BF);
# the `text` theme never draws one.
#
# Skips (test_skip) without python3.

source (dirname (status -f))/../lib/test_helpers.fish

set -g __pty $__gpy_root/tests/lib/pty_session.py

function __write_config_theme --argument-names theme
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '[ui]\ntheme = "%s"\nenabled_segments = ["directory"]\n' $theme >$XDG_CONFIG_HOME/gpy/config.toml
end

function has_chevrons
    string match -qr "[\x{e0b0}-\x{e0bf}]" -- $argv
end

# Bytes the session wrote after $offset.
function transcript_after --argument-names dir offset
    tail -c +(math $offset + 1) $dir/transcript | string collect
end

function test_new_shell_after_offline_edit
    print_test_header "E2E Test: New Shell Picks Up an Offline Config Edit"
    init_test_env
    __write_config_theme default
    if not start_test_agent
        fail "Agent Start" "gpy-agent failed to start"
        return 1
    end
    set -l plain $__gpy_test_tmp_dir/plain
    mkdir -p $plain
    set -l render (fish --no-config -c 'source $argv[1]/fish/functions/fish_prompt.fish; cd $argv[2]; fish_prompt' -- $__gpy_root $plain 2>/dev/null | string collect)
    if has_chevrons "$render"
        pass "the default theme draws chevrons (control)"
    else
        fail "the default theme draws chevrons (control)" (string escape -- "$render")
    end
    stop_test_agent

    __write_config_theme text
    set render (fish --no-config -c 'source $argv[1]/fish/functions/fish_prompt.fish; cd $argv[2]; fish_prompt' -- $__gpy_root $plain 2>/dev/null | string collect)
    if not has_chevrons "$render"; and string match -q '*plain*' -- "$render"
        pass "a new shell draws the new theme on its first prompt"
    else
        fail "a new shell draws the new theme on its first prompt" (string escape -- "$render")
    end
    return 0
end

function test_existing_shell_after_offline_edit
    print_test_header "E2E Test: Existing Shell Picks Up an Offline Config Edit"
    init_test_env
    __write_config_theme default
    if not start_test_agent
        fail "Agent Start" "gpy-agent failed to start"
        return 1
    end
    set -l plain $__gpy_test_tmp_dir/plain
    mkdir -p $plain

    set -l dir $__gpy_test_tmp_dir/session
    env GPY_E2E_ROOT=$__gpy_root GPY_E2E_CWD=$plain \
        python3 $__pty start $dir -- fish --no-config -i -C '
        source $GPY_E2E_ROOT/fish/conf.d/gpy_init.fish
        source $GPY_E2E_ROOT/fish/functions/fish_prompt.fish
        cd $GPY_E2E_CWD
    '
    function __stop_session --inherit-variable dir
        python3 $__pty stop $dir >/dev/null 2>&1
    end
    test_track_cleanup __stop_session

    function __default_drawn --inherit-variable dir
        has_chevrons (transcript_after $dir 0)
    end
    if not poll_until 10 __default_drawn
        fail "Existing shell draws the default theme" "no chevron within 10 s"
        __stop_session
        return 1
    end
    pass "Existing shell draws the default theme"

    stop_test_agent
    set -l off (python3 $__pty size $dir)
    __write_config_theme text
    if not start_test_agent
        fail "Agent restart" "gpy-agent failed to start"
        __stop_session
        return 1
    end

    # No keystroke: the restarted agent's ring makes the shell repaint, and
    # that repaint must be the text theme.
    function __text_repaint --inherit-variable dir --inherit-variable off
        set -l after (transcript_after $dir $off)
        string match -q '*plain*' -- "$after"; and not has_chevrons "$after"
    end
    if poll_until 10 __text_repaint
        pass "Existing shell repaints with the new theme after restart"
    else
        fail "Existing shell repaints with the new theme after restart" (transcript_after $dir $off | string escape)
    end
    __stop_session
    return 0
end

if not command -q python3
    test_skip "python3 is required to drive a pseudo-terminal"
end

test_new_shell_after_offline_edit
cleanup_test_files
test_existing_shell_after_offline_edit
cleanup_test_files
test_finish "theme refresh after offline edit"
