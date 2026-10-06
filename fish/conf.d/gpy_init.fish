# SPDX-License-Identifier: GPL-3.0-or-later
# Safe GPY entry point with fail-safe mechanisms

# GPY is a prompt: non-interactive shells (fish -c, scripts, the supervisor's
# own children) must not load it, define its handlers, or spawn an agent (#769).
# fish/core/init.fish is deliberately NOT gated; tests and ci-bench source it.
status is-interactive; or return

# Whether this file ends up in "disabled mode" — by user choice or because the
# install is broken — and therefore has to leave fish_prompt's globals in a
# safe, defined state before returning. See the block at the bottom of this
# file, which is the single place those defaults are set (#452, #457).
set -l gpy_needs_prompt_defaults 0

if test "$GPY_AGENT_ENABLED" = 0 -a "$GPY_AGENT_SUPERVISOR_ENABLED" = 0
    # GPY is completely disabled by the user. This branch intentionally never
    # resolves the install dir, so it stays correct even when the install at
    # $gpy_install_dir is missing, broken, or stale relative to this file.
    set gpy_needs_prompt_defaults 1
else
    # Set proper defaults (enabled by default)
    set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
    set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

    # Find GPY installation
    # (This file is installed at ~/.config/fish/conf.d/gpy_init.fish)
    # Configuration files are at ~/.config/gpy/ (themes, config)
    # Fish implementation is at ~/.config/fish/gpy/ (core, segments, functions)

    # Determine Fish config directory
    set -l fish_config_dir
    if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
        set fish_config_dir "$XDG_CONFIG_HOME/fish"
    else
        set fish_config_dir "$HOME/.config/fish"
    end

    set -l gpy_install_dir "$fish_config_dir/gpy"

    # Validate installation before loading
    if not test -f "$gpy_install_dir/core/init.fish"
        echo "gpy[init]: core files not found at $gpy_install_dir - GPY disabled" >&2
        set -gx GPY_AGENT_ENABLED 0
        set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
        set gpy_needs_prompt_defaults 1
    else if not source "$gpy_install_dir/core/init.fish" 2>/dev/null
        # Safe loading with error handling
        echo "gpy[init]: failed to load GPY safely - using fallback mode" >&2
        set -gx GPY_AGENT_ENABLED 0
        set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
        set gpy_needs_prompt_defaults 1
    end
end

# fish/functions/fish_prompt.fish is autoloaded by Fish independently of this
# file (it lives under functions/, which Fish's autoloader scans regardless of
# whether this file — or core/init.fish — ever ran) and reads these globals on
# every render. Without them it hits unset-variable errors on every prompt
# (#452: the user-disabled route; #457: the broken-install and fallback-mode
# routes, where fish_prompt.fish is installed and only GPY's own init failed).
#
# These mirror the same guarded defaults core/init.fish's own disabled
# early-exit sets, since that file is never reached — or never finished — on
# any of these paths; each is guarded so a real theme export always wins.
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
    # Mirror __gpy_initialize_icons' nerd/ascii split: constants.fish is not
    # sourced on this path, so the literals are inlined, but an ascii-preferring
    # user must not get nerd glyphs just because GPY is disabled.
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
