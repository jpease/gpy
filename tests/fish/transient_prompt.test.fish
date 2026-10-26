#!/usr/bin/env fish
# Transient prompt (ui.transient_prompt, Fish 4.2+). Fish re-runs fish_prompt
# with --final-rendering after Enter and keeps that rendering in the
# scrollback:
#   - on: the clock's time, then the prompt symbol alone (no status
#     indicator, no other segment). Colors are not asserted: set_color prints
#     nothing without a terminal;
#   - a symbol that opens with a space gets no second one;
#   - off: the full prompt again, so the clock shows the time Enter was
#     pressed.
# __gpy_sync_fish_transient_prompt turns fish_transient_prompt on only when
# the final rendering changes something, and only ever erases its own value.

source (dirname (status --filename))/../lib/test_helpers.fish

init_test_env; or exit 1

source fish/core/init.fish
source fish/functions/fish_prompt.fish

function plain
    string replace -ra '\e\[[0-9;]*m' '' -- $argv | string collect
end
function final_after_failure
    false
    fish_prompt --final-rendering 2>/dev/null | string collect
end

set -g __enabled_segments clock
set -g __clock_format ""
set -g __time_format 12
set -g __clock_show_seconds 0
set -g __color_clock_bg blue
set -g __color_clock_fg yellow
set -g __prompt_time_pad " "
set -g __gpy_is_root 0
set -g __gpy_add_newline 0
set -g __gpy_two_line 1
set -g __prompt_color cyan
set -g __icon_prompt "❯"
set -g __root_prompt_color red
set -g __icon_root_prompt "!❯!"
set -g __icon_status_ok STATUSOK
set -g __icon_status_fail STATUSFAIL
set -g GPY_SHOW_STATUS 1
set -g __gpy_char_cache_key ""
function __gpy_request_character
end

# --- on ----------------------------------------------------------------------
set -gx GPY_UI_TRANSIENT_PROMPT 1
set -l out (final_after_failure)
set -l time_re '^ [0-9]{1,2}:[0-9]{2} [AP]M '
string match -qr -- "$time_re❯ \$" (plain $out)
check "on: the time, then the fallback symbol only" 1 (test $status -eq 0; and echo 1)
check "on: no status indicator" 1 (string match -q '*STATUS*' -- $out; or echo 1)

set -g __gpy_is_root 1
string match -qr -- "$time_re!❯! \$" (plain (final_after_failure))
check "root: the root symbol" 1 (test $status -eq 0; and echo 1)
set -g __gpy_is_root 0

# An agent character that opens with a space, as the default theme's does.
function __gpy_request_character
    printf ' AGENTCHAR '
end
set -g __gpy_char_cache_key ""
string match -qr -- "$time_re""AGENTCHAR \$" (plain (final_after_failure))
check "on: a symbol opening with a space gets no second one" 1 (test $status -eq 0; and echo 1)
function __gpy_request_character
end
set -g __gpy_char_cache_key ""

set -g __enabled_segments
string match -q -- "❯ " (plain (final_after_failure))
check "on, no clock: the symbol alone" 1 (test $status -eq 0; and echo 1)

# --- off ---------------------------------------------------------------------
# The two renders can straddle a minute boundary, so the time is replaced by a
# placeholder before comparing; each render must still carry one.
set -gx GPY_UI_TRANSIENT_PROMPT 0
set -g __enabled_segments clock
set -l clock_re '[0-9]{1,2}:[0-9]{2} [AP]M'
set -l final (plain (final_after_failure) | string replace -r -- $clock_re TIME | string collect)
set -l full (plain (false; fish_prompt 2>/dev/null | string collect) | string replace -r -- $clock_re TIME | string collect)
check "off: the final rendering is the full prompt" 1 (test "$final" = "$full"; and string match -q '*TIME*STATUSFAIL*' -- $final; and echo 1)

# --- fish_transient_prompt ---------------------------------------------------
set -e -g fish_transient_prompt
set -e -g __gpy_owns_fish_transient_prompt
set -g __gpy_fish_transient_capable 0
set -gx GPY_UI_TRANSIENT_PROMPT 1
__gpy_sync_fish_transient_prompt
check "Fish before 4.2: left unset" 1 (set -q fish_transient_prompt; or echo 1)

set -g __gpy_fish_transient_capable 1
__gpy_sync_fish_transient_prompt
check "on: set to 1" 1 (test "$fish_transient_prompt" = 1; and echo 1)

set -gx GPY_UI_TRANSIENT_PROMPT 0
__gpy_sync_fish_transient_prompt
check "off with the clock: still 1, to refresh the time" 1 (test "$fish_transient_prompt" = 1; and echo 1)

set -g __enabled_segments directory
__gpy_sync_fish_transient_prompt
check "off without the clock: GPY's value is erased" 1 (set -q fish_transient_prompt; or echo 1)

set -g fish_transient_prompt 1
__gpy_sync_fish_transient_prompt
check "off without the clock: the user's own value is kept" 1 (test "$fish_transient_prompt" = 1; and echo 1)

functions -e __gpy_request_character
test_finish "transient prompt"
