# SPDX-License-Identifier: GPL-3.0-or-later

# Re-read config.toml into the running shell. Always asks `gpy-agent theme
# export` (never the cache file, which may be stale), so it works whether the
# agent is running, disabled or stopped. Silent on success. An invalid
# config.toml applies the built-in defaults and the agent's warning shows.
function gpy-prompt-reload --description 'reload the GPY theme and config into this shell'
    if not __gpy_reload_theme fresh
        echo "gpy: theme reload failed; run gpy doctor" >&2
        return 1
    end
end
