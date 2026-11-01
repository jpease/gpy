# SPDX-License-Identifier: GPL-3.0-or-later

# Deprecated name of gpy-prompt-debug, kept for one minor release.
function prompt-debug --description 'deprecated: use gpy-prompt-debug'
    if not set -q __gpy_warned_prompt_debug
        set -g __gpy_warned_prompt_debug 1
        echo "gpy: prompt-debug is deprecated; use gpy-prompt-debug instead" >&2
    end
    gpy-prompt-debug $argv
end
