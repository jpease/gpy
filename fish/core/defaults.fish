# SPDX-License-Identifier: GPL-3.0-or-later

# Safe prompt globals for disabled mode: fish_prompt.fish autoloads regardless
# of init, so these keep it rendering without unset-variable diagnostics.
# constants.fish is not sourced here; the ascii icon preference is honoured anyway.
# No segment file is loaded in disabled mode, so the list stays empty.
set -q __enabled_segments; or set -g __enabled_segments
set -q __prompt_icons; or set -g __prompt_icons nerd
set -q __prompt_theme; or set -g __prompt_theme default

set -q __gpy_is_root; or begin
    set -g __gpy_is_root 0
    test (id -u) -eq 0; and set -g __gpy_is_root 1
end
set -q __prompt_color; or set -g __prompt_color green
set -q __root_prompt_color; or set -g __root_prompt_color red
set -q __icon_root_prompt; or set -g __icon_root_prompt "!❯!"
if test "$__prompt_icons" = nerd
    set -q __icon_prompt; or set -g __icon_prompt "❯"
    set -q __icon_status_ok; or set -g __icon_status_ok "✔"
    set -q __icon_status_fail; or set -g __icon_status_fail "✖"
else
    set -q __icon_prompt; or set -g __icon_prompt ">"
    set -q __icon_status_ok; or set -g __icon_status_ok ok
    set -q __icon_status_fail; or set -g __icon_status_fail err
end
