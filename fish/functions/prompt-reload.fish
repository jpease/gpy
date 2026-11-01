# SPDX-License-Identifier: GPL-3.0-or-later

# Deprecated name of gpy-prompt-reload, kept for one minor release.
function prompt-reload --description 'deprecated: use gpy-prompt-reload'
    if not set -q __gpy_warned_prompt_reload
        set -g __gpy_warned_prompt_reload 1
        echo "gpy: prompt-reload is deprecated; use gpy-prompt-reload instead" >&2
    end
    gpy-prompt-reload $argv
end
