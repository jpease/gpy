#!/usr/bin/env fish
# Transient prompt (ui.transient_prompt, Fish 4.2+). Fish re-runs fish_prompt
# with --final-rendering after Enter and keeps that rendering in the
# scrollback; `gpy prompt fish` prints the prompt, 0x1F, then that line:
#   - on: the clock's time, then the prompt symbol alone (no other segment);
#   - off with the clock: the full prompt again, so the clock shows the time
#     Enter was pressed;
#   - off without the clock: nothing to swap in.
# fish_prompt sets fish_transient_prompt only on a capable Fish
# (__gpy_fish_transient_capable, 4.2+) and only while the binary gives it a
# line to swap in, and only ever erases a value it set itself. The capability
# comes from the __gpy_test_fish_version hook, so this runs on any Fish.

source (dirname (status --filename))/../lib/test_helpers.fish

init_test_env; or exit 1
set -gx GPY_AGENT_ENABLED 0
mkdir -p $XDG_CONFIG_HOME/gpy

function config --argument-names transient segments
    printf '%s\n' '[ui]' "transient_prompt = $transient" "enabled_segments = [$segments]" >$XDG_CONFIG_HOME/gpy/config.toml
end

function plain
    string replace -ra '\e\[[0-9;]*m' '' -- $argv | string collect
end

# Run $argv[1] in an interactive Fish with the integration loaded, faking
# Fish $argv[2] (empty: the real version).
function in_shell --argument-names code fish_ver
    fish --no-config -i -c '
        test -n "$argv[3]"; and set -g __gpy_test_fish_version $argv[3]
        source $argv[1]/fish/conf.d/gpy_init.fish
        source $argv[1]/fish/functions/fish_prompt.fish
        eval $argv[2]' -- $__gpy_root $code "$fish_ver" </dev/null 2>&1
end

set -l time_re '[0-9]{1,2}:[0-9]{2}( [AP]M)?'

# --- the final rendering -------------------------------------------------------
config true '"clock", "directory"'
set -l final (plain (in_shell 'false; fish_prompt --final-rendering' | string collect))
check "on: the time, then the symbol alone" 1 (string match -qr -- "^\s*$time_re ❯ \$" "$final"; and echo 1)
check "on: no other segment" 1 (string match -q -- '*gpy*' "$final"; or echo 1)

config true '"directory"'
set final (plain (in_shell 'false; fish_prompt --final-rendering' | string collect))
check "on, no clock: the symbol alone" 1 (string match -qr -- '^\s*❯ $' "$final"; and echo 1)

# The two renders can straddle a minute boundary, so the time is replaced by
# a placeholder before comparing; each render must still carry one.
config false '"clock", "directory"'
set -l both (in_shell 'false; fish_prompt; printf "\n--SPLIT--\n"; false; fish_prompt --final-rendering' | string collect)
set -l parts (string split -- \n--SPLIT--\n (plain $both | string replace -ar -- $time_re TIME | string collect))
check "off with the clock: the final rendering is the full prompt" 1 (test "$parts[1]" = "$parts[2]"; and string match -q '*TIME*' -- $parts[1]; and echo 1)

config false '"directory"'
set final (in_shell 'fish_prompt --final-rendering' | string collect)
check "off without the clock: an empty final rendering" "" "$final"

# --- fish_transient_prompt ---------------------------------------------------
set -l report 'fish_prompt >/dev/null; set -q fish_transient_prompt; and echo "ftp=$fish_transient_prompt"; or echo ftp=unset'

config true '"clock"'
check "Fish before 4.2: left unset" ftp=unset (in_shell $report 4.1.0)
check "on: set to 1" ftp=1 (in_shell $report)

config false '"clock"'
check "off with the clock: 1, to refresh the time" ftp=1 (in_shell $report)

config false '"directory"'
check "off without the clock: never set" ftp=unset (in_shell $report)
check "off without the clock: the user's own value is kept" ftp=0 (in_shell "set -g fish_transient_prompt 0; $report")

# GPY's own value is erased once the binary has nothing to swap in.
set -l cfg $XDG_CONFIG_HOME/gpy/config.toml
set -l toggle "fish_prompt >/dev/null; echo first=\$fish_transient_prompt; printf '%s\n' '[ui]' 'transient_prompt = false' 'enabled_segments = [\"directory\"]' >$cfg; $report"
config true '"clock"'
set -l out (in_shell $toggle | string collect)
check "on: GPY sets it" 1 (string match -q '*first=1*' -- "$out"; and echo 1)
check "then off without the clock: GPY's value is erased" 1 (string match -q '*ftp=unset*' -- "$out"; and echo 1)

test_finish "transient prompt"
