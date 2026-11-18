# shellcheck shell=bash
# tests/lib/shell_render_only.bash - source after bash/gpy.bash.
#
# Every prompt goes through `gpy prompt bash` (#1247); the shell renderer is
# its fallback until the cutover deletes it. Tests that pin that renderer
# (stubbed theme state, mocked agents, counted forks) source this file so
# precmd falls back to it, as when the binary is missing. The binary path has
# its own coverage: tests/bash/prompt_exec_parity.test.bash.
__gpy_render_prompt_exec() { return 1; }
