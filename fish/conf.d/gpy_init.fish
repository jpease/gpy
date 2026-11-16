# SPDX-License-Identifier: GPL-3.0-or-later

# Non-interactive shells (fish -c, scripts) must not load GPY.
# core/init.fish itself is not gated: tests and ci-bench source it.
status is-interactive; or return

# A retry (re-sourcing this file after fixing the cause) starts clean.
set -e __gpy_load_failed

# Disabled mode must still leave fish_prompt's globals defined (see bottom).
set -l gpy_needs_prompt_defaults 0

# The tree this file belongs to: conf.d sits beside core/.
set -l gpy_install_dir (path normalize (status dirname)/..)

set -l gpy_flags_loaded 0
test -f "$gpy_install_dir/core/flags.fish"; and source "$gpy_install_dir/core/flags.fish"; and set gpy_flags_loaded 1

if test $gpy_flags_loaded -eq 1; and not __gpy_runtime_root >/dev/null
    # No usable XDG_RUNTIME_DIR, XDG_CACHE_HOME or HOME: the agent refuses to
    # run, and there is no shared /tmp fallback, so GPY stays off this session.
    test "$GPY_DEBUG" = 1; and echo "gpy[init]: debug: no runtime directory (set XDG_RUNTIME_DIR, XDG_CACHE_HOME or HOME to an absolute path) - GPY disabled" >&2
    set -g __gpy_load_failed 1
    set gpy_needs_prompt_defaults 1
else if test $gpy_flags_loaded -eq 1; and not __gpy_agent_enabled; and not __gpy_supervisor_enabled
    set gpy_needs_prompt_defaults 1
else
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
