# SPDX-License-Identifier: GPL-3.0-or-later

# Non-interactive shells (fish -c, scripts) must not load GPY.
# core/init.fish itself is not gated: tests and ci-bench source it.
status is-interactive; or return

# A retry (re-sourcing this file after fixing the cause) starts clean.
set -e __gpy_load_failed

# Disabled mode must still leave fish_prompt's globals defined (see bottom).
set -l gpy_needs_prompt_defaults 0

set -l fish_config_dir
if set -q XDG_CONFIG_HOME; and test -n "$XDG_CONFIG_HOME"
    set fish_config_dir "$XDG_CONFIG_HOME/fish"
else
    set fish_config_dir "$HOME/.config/fish"
end

set -l gpy_install_dir "$fish_config_dir/gpy"

if test -f "$gpy_install_dir/core/flags.fish"; and source "$gpy_install_dir/core/flags.fish"; and not __gpy_agent_enabled; and not __gpy_supervisor_enabled
    set gpy_needs_prompt_defaults 1
else
    set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
    set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

    if not test -f "$gpy_install_dir/core/init.fish"
        echo "gpy[init]: core files not found at $gpy_install_dir - GPY disabled" >&2
        set -g __gpy_load_failed 1
        set gpy_needs_prompt_defaults 1
    else if not source "$gpy_install_dir/core/init.fish"
        echo "gpy[init]: GPY failed to load; using the fallback prompt" >&2
        set -g __gpy_load_failed 1
        set gpy_needs_prompt_defaults 1
    end
end

# fish_prompt.fish autoloads regardless of init, so set guarded defaults when init didn't run.
if test $gpy_needs_prompt_defaults -eq 1; and test -f "$gpy_install_dir/core/defaults.fish"
    source "$gpy_install_dir/core/defaults.fish"
end
