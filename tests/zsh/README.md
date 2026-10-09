# Zsh Tests

Every `*.test.zsh` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow. All files here are
shell-integration tests of `zsh/gpy.zsh` (the repository-policy checks live
in `tests/bash/`, see its README for the split).

**Cross-shell behaviour is not tested here.** `tests/bash/shell_contract.test.bash`
runs one scenario table (`tests/fixtures/shell_scenarios/scenarios.tsv`) through
Fish, Bash and Zsh and compares prompts and side effects (agent starts, oneshot
forks, IPC requests, registrations). A Zsh behaviour that is meant to match the
other shells (a #841-#845 style drift fix) adds a row there, not a Zsh-only file;
see "The cross-shell contract harness" in `tests/bash/README.md`.

| Tier | Files | Agent? |
|---|---|---|
| Live daemon, real terminal | `e2e_agent_autostart`, `e2e_git_live_content`, `e2e_reregister_after_restart`, `e2e_config_reload`, `e2e_exec_survives_doorbell` | real `gpy-agent`, real `zsh -i` on a pty |
| Live daemon, no terminal | `e2e_agent_down_stale_git` (agent down, then up), `e2e_git_cold_miss_bounded_sync`, `e2e_git_variant_fallback_bounded_sync`, `e2e_lang_variant_fallback_bounded_refresh`, `maybe_refresh_nonblocking`, `path_parity` (`gpy debug paths` vs `__gpy_debug_paths`, cache key against a file the agent wrote), `prompt_text_literal` (directory and branch names with `$(…)`, backticks, `%` and `!` show literally after `print -rP`: fresh render, oneshot fallback, instant cache), `prompt_nonprinting` (every agent SGR escape in PROMPT sits inside `%{ %}`: fresh render, instant cache) | real `gpy-agent`, functions called directly |
| Function-level | `basic`, `integration`, `cache_key_vectors` (`__gpy_path_to_cache_key` matches the agent encoder on every row of `tests/fixtures/cache_key_vectors.tsv`, #705), `xdg_absoluteness_vectors` (XDG_* values are honoured iff absolute, per `tests/fixtures/xdg_absoluteness_vectors.tsv`, #774), `venv_forwarding_vectors` (`lang` requests forward `$VIRTUAL_ENV`, else a non-base `$CONDA_PREFIX`, per `tests/fixtures/venv_forwarding_vectors.tsv`, #729), `json_escape`, `json_flags_tail`, `load_theme_cache`, `instant_cache_status`, `is_first_instant_cache`, `language_marker_prefilter` (the pre-filter accepts every marker in the agent-exported `__gpy_lang_marker_files`, rejects an empty dir, defers when unset, #785), `oneshot_budget`, `agent_disabled_prompt`, `parity`, `prompt_dispatch_positions`, `omitted_segment_positions` (a language cold miss is dropped before positions are assigned, so its neighbour keeps the first/last form), `segment_bg_export`, `missing_core_file_disables_cleanly`, `duration_integer_ms` (the duration and its request are integer milliseconds) | none (`integration` runs with the supervisor off and a socket that does not exist) |
| Shell-drawn segments | `local_segment_renderer` (the status pill, local clock, hostname/username fallback and root prompt go through `zsh/core/renderer.zsh`: the render loop hands the next segment the pill's ok/fail background as `prev_bg`, root draws `__icon_root_prompt` in `__root_prompt_color` and never asks the agent for a character, hostile hostnames/usernames/icons show literally, the color table maps names, bright names, hex and 0-255; #844). The Fish/Bash/Zsh byte comparison is `tests/bash/shell_contract.test.bash` | none: stubbed character request, supervisor off, socket that does not exist |
| Completion widgets | `completions` | a nested zsh via `zsh/zpty` |
| Hostile environment | `hostile_env` (fresh child zsh whose rc builds a hostile environment before sourcing `gpy.zsh`; asserts the user's environment survived), `cache_dir_no_leak` (the private `$TMPDIR/gpy_cache_*` render-cache dir does not leak on re-source, `exec zsh` or a killed shell, and a live shell's dir is never removed by a sibling) | none: supervisor off, socket that does not exist |
| IPC edge cases | `ipc_partial_response`, `ipc_no_double_send_on_slow_reply`, `ipc_slow_reply_transports` (socat and nc follow the same connected-but-late policy, #845), `ipc_nc_requires_unix_support` (an `nc` whose help lacks `-U` is no client, #850), `env_knobs_honoured` (`GPY_IPC_TIMEOUT_MS` and the TTL variables, #845), `protocol_version_mismatch`, `ipc_nc_fallback_timeout`, `ipc_send_preserves_escapes` (backslashes and quotes reach the agent and come back verbatim), `ipc_timeout_honors_budget` (the zsocket and socat transports honour `GPY_IPC_TIMEOUT_MS`; a slow reply never forks a oneshot), `register_rejects_error_reply` (an `{"error":…}` reply to register does not count as registered), `workspace_error_keeps_registration` (a workspace error other than "not registered" keeps the registration) | a python3 fake listener |
| Supervisor config | `supervisor_respects_config` (the per-prompt check honours `[agent.supervisor]` `enabled`, `check_interval_seconds` and `max_restart_attempts` from the theme export, #762) | a stub `gpy-agent` on `PATH` |

The agent and supervisor flags come from `config.toml` through the theme
export, which loads after `gpy.zsh` starts sourcing, so exporting
`GPY_AGENT_ENABLED=0` or `GPY_AGENT_SUPERVISOR_ENABLED=0` does not stop
`gpy.zsh` from starting an agent. `[agent.supervisor] enabled = false` alone
never does either: it only turns off restarts (#842), and the startup start
depends on `[agent] enabled`. Tests that must not start an agent source
`tests/lib/supervisor_off.zsh` (a throwaway config with `[agent] enabled =
false` and `[agent.supervisor] enabled = false`), write those keys into their
own sandbox `config.toml` (#762), or source with no `gpy-agent` on `PATH`
(`shell_e2e_agent_free_begin` / `shell_e2e_agent_free_end`).

## The live-daemon harness

`tests/lib/shell_e2e.sh` is shared with the Bash suite and documented in
`tests/bash/README.md`. Source it from zsh with sh semantics so its functions
keep sh word-splitting:

```zsh
ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
shell_e2e_init "$ROOT"
shell_e2e_spawn_client zsh "$ROOT"
```

The skip contract (`test_skip`, exit 0 locally and exit 1 under `CI`, #650)
comes from the same file.

## Running

```bash
just check-shell                            # everything, as the gate runs it
zsh tests/zsh/e2e_agent_autostart.test.zsh  # one file
```
