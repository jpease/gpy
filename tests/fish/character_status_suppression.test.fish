#!/usr/bin/env fish
# Verify the legacy status indicator is suppressed when the character segment is
# agent-rendered (the exit-colored `❯` conveys success/failure), and still shown
# when the agent returns nothing (legacy/fallback symbol). Post-#199 the behavior
# is driven by whether the agent actually renders the character, not by a static
# `__gpy_character_format` toggle.

source (dirname (status --current-filename))/../support/setup_test_env.fish

function test_status_indicator_gated_by_agent_render
    source fish/core/init.fish
    # Load the prompt function from the repo (not any autoloaded/installed copy).
    source fish/functions/fish_prompt.fish

    # Minimal, deterministic prompt environment (no enabled segments).
    set -g __enabled_segments
    set -g __gpy_is_root 0
    set -g __prompt_color white
    set -g __icon_prompt "❯"
    set -g __icon_status_ok STATUSOK
    set -g __icon_status_fail STATUSFAIL
    set -g GPY_SHOW_STATUS 1

    # Fallback path: the agent renders nothing → the separate status indicator is
    # shown (legacy symbol can't convey success/failure on its own).
    function __gpy_request_character
    end
    set -l fallback (fish_prompt 2>/dev/null | string collect)
    if not string match -q "*STATUSOK*" -- $fallback
        echo "FAIL: status indicator missing when agent returns no character (regression)"
        exit 1
    end
    echo "✓ status indicator shown when agent returns no character (no regression)"

    # Agent-rendered path: the agent returns a character → the status indicator is
    # dropped (the exit-colored `❯` conveys success/failure instead).
    function __gpy_request_character
        printf AGENTCHAR
    end
    set -l rendered (fish_prompt 2>/dev/null | string collect)
    if string match -q "*STATUSOK*" -- $rendered
        echo "FAIL: status indicator still shown when character is agent-rendered"
        exit 1
    end
    if not string match -q "*AGENTCHAR*" -- $rendered
        echo "FAIL: agent-rendered character symbol missing from prompt"
        exit 1
    end
    echo "✓ status indicator suppressed when character is agent-rendered"

    functions -e __gpy_request_character
end

test_status_indicator_gated_by_agent_render
