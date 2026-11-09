#!/usr/bin/env fish
# Test: the shell-drawn prompt symbol and transient clock map the theme
# keywords `transparent`, `match_bg`, `match_text` (and empty) to the terminal
# default instead of passing them to `set_color`, which rejects them and would
# print `Unknown color` on every prompt render.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/renderer.fish"
source "$repo_root/fish/segments/clock.fish"
source "$repo_root/fish/functions/fish_prompt.fish"

# set_color emits nothing on a dumb terminal; the real-colour case needs SGR.
set -gx TERM xterm-256color

set -g __gpy_is_root 0
set -g __gpy_char_agent_rendered 0
set -g __icon_prompt ❯
set -g __icon_root_prompt "#"
set -g __enabled_segments clock
set -g __gpy_add_newline 0
function __gpy_resolve_prompt_character
end

set -l err_file (mktemp)

function run_case --argument-names label err_file
    set -l out ($argv[3..-1] 2>$err_file | string collect)
    set -l err (cat $err_file | string collect)
    if string match -q '*Unknown color*' -- "$err"
        fail "$label (stderr: $err)"
    else
        pass "$label"
    end
    set -g __case_out $out
end

for color in transparent match_bg match_text ''
    set -g __gpy_is_root 0
    set -g __prompt_color $color
    run_case "prompt symbol: '$color' __prompt_color emits no set_color error" $err_file __gpy_print_prompt_symbol
    check "prompt symbol: '$color' still prints the icon" 1 (string match -q '*❯*' -- "$__case_out"; and echo 1; or echo 0)
end

set -g __gpy_is_root 1
for color in transparent match_bg
    set -g __root_prompt_color $color
    run_case "root prompt symbol: $color __root_prompt_color emits no set_color error" $err_file __gpy_print_prompt_symbol
    check "root prompt symbol: $color still prints the icon" 1 (string match -q '*#*' -- "$__case_out"; and echo 1; or echo 0)
end
set -g __gpy_is_root 0

for color in transparent match_bg
    set -g __color_clock_fg $color
    run_case "transient prompt: $color __color_clock_fg emits no set_color error" $err_file __gpy_transient_prompt 0
    check "transient prompt: $color still prints the icon" 1 (string match -q '*❯*' -- "$__case_out"; and echo 1; or echo 0)
end

set -g __prompt_color green
run_case "prompt symbol: a real colour reaches set_color" $err_file __gpy_print_prompt_symbol
check "prompt symbol: green emits SGR 32" 1 (string match -q -- "*"(printf '\e[32m')"*❯*" "$__case_out"; and echo 1; or echo 0)

rm -f $err_file
test_finish "prompt symbol colour tests"
