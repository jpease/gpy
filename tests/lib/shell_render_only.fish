# tests/lib/shell_render_only.fish - source after fish/functions/fish_prompt.fish.
#
# Every prompt goes through `gpy prompt fish` (#1247); fish_prompt's own
# renderer is its fallback until the cutover deletes it. Tests that pin that
# renderer (stubbed segments, mocked agents, counted forks) source this file
# so fish_prompt falls back to it, as when the binary is missing. The binary
# path has its own coverage: tests/bash/fish_prompt_exec_parity.test.bash.
# The fish_prompt event then runs the script segments in the shell too.
set -g __gpy_exec_declined 1
function __gpy_render_prompt_exec
    return 1
end
