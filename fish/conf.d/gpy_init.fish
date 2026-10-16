# SPDX-License-Identifier: GPL-3.0-or-later

# Non-interactive shells (fish -c, scripts, supervisor children) must not load GPY.
# core/init.fish itself is not gated: tests and ci-bench source it.
status is-interactive; or return

# Disabled mode must still leave fish_prompt's globals defined (see bottom).
set -l gpy_needs_prompt_defaults 0

if test "$GPY_AGENT_ENABLED" = 0 -a "$GPY_AGENT_SUPERVISOR_ENABLED" = 0
    set gpy_needs_prompt_defaults 1
else
    set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
    set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end

    set -l gpy_install_dir "$fish_config_dir/gpy"

    if not test -f "$gpy_install_dir/core/init.fish"
        echo "gpy[init]: core files not found at $gpy_install_dir - GPY disabled" >&2
        set -gx GPY_AGENT_ENABLED 0
        set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
        set gpy_needs_prompt_defaults 1
    else if not source "$gpy_install_dir/core/init.fish" 2>/dev/null
        echo "gpy[init]: failed to load GPY safely - using fallback mode" >&2
        set -gx GPY_AGENT_ENABLED 0
        set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
        set gpy_needs_prompt_defaults 1
    end
end

# fish_prompt.fish autoloads regardless of init, so set guarded defaults when init didn't run.
if test $gpy_needs_prompt_defaults -eq 1
    set -q __enabled_segments; or set -g __enabled_segments clock duration directory
    set -q __prompt_icons; or set -g __prompt_icons nerd
    set -q __prompt_theme; or set -g __prompt_theme default

    set -q __gpy_is_root; or begin
        set -g __gpy_is_root 0
        test (id -u) -eq 0; and set -g __gpy_is_root 1
    end
    set -q __prompt_color; or set -g __prompt_color green
    set -q __root_prompt_color; or set -g __root_prompt_color red
    set -q __icon_root_prompt; or set -g __icon_root_prompt "!❯!"
    # constants.fish is not sourced here; respect ascii icon preference anyway.
    if test "$__prompt_icons" = nerd
        set -q __icon_prompt; or set -g __icon_prompt "❯"
        set -q __icon_status_ok; or set -g __icon_status_ok "✔"
        set -q __icon_status_fail; or set -g __icon_status_fail "✖"
    else
        set -q __icon_prompt; or set -g __icon_prompt ">"
        set -q __icon_status_ok; or set -g __icon_status_ok ok
        set -q __icon_status_fail; or set -g __icon_status_fail err
    end
end
