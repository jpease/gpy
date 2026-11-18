# tests/lib/shell_render_only.zsh - source after zsh/gpy.zsh.
#
# Every prompt goes through `gpy prompt zsh` (#1247); the shell renderer is
# its fallback until the cutover deletes it. Tests that pin that renderer
# (stubbed theme state, mocked agents, counted forks) source this file so
# precmd and repaints fall back to it, as when the binary is missing. The
# binary path has its own coverage: tests/zsh/prompt_exec_parity.test.zsh.
function __gpy_render_prompt_exec() { return 1 }
