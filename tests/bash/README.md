# Bash Tests

Every `*.test.bash` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow.

Two kinds of file live here, and the split matters when you add one:

## Shell-integration tests

Exercise `bash/gpy.bash` and its segments.

| Tier | Files | Agent? |
|---|---|---|
| Live daemon, real terminal | `e2e_agent_autostart`, `e2e_git_live_content`, `e2e_reregister_after_restart`, `e2e_config_reload`, `e2e_exec_survives_doorbell` | real `gpy-agent`, real `bash -i` on a pty |
| Live daemon, no terminal | `e2e_agent_down_stale_git` (agent down, then up), `path_parity` (`gpy debug paths` vs `__gpy_debug_paths`, cache key against a file the agent wrote), `prompt_text_literal` (directory and branch names with `$(…)`, backticks, `\` and `!` show literally in the expanded PS1: fresh render, oneshot fallback, instant cache), `prompt_nonprinting` (every agent SGR escape in PS1 sits inside `\[ \]`: fresh render, instant cache) | real `gpy-agent`, functions called directly |
| Interactive, no agent | `debug_trap_preserves_last_arg` (`$_` survives the DEBUG trap: `mkdir -p d && cd $_`), `doorbell_no_reentry` (on a pty: a SIGURG burst never nests renders or blocks the shell, `wait` is not interrupted, a reload flag is applied by the next prompt), `duration_measures_command_line` (`__gpy_cmd_duration` spans the whole command line: other `PROMPT_COMMAND` entries, lists, loops, a SIGURG, idle time, string and array `PROMPT_COMMAND`) | none: a real `bash -i` fed on stdin or on a pty, supervisor off, socket that does not exist |
| Function-level | `basic`, `integration`, `debug_trap`, `duration_math`, `cache_key_vectors` (`__gpy_path_to_cache_key` matches the agent encoder on every row of `tests/fixtures/cache_key_vectors.tsv`, #705), `xdg_absoluteness_vectors` (XDG_* values are honoured iff absolute, per `tests/fixtures/xdg_absoluteness_vectors.tsv`, #774), `venv_forwarding_vectors` (`lang` requests forward `$VIRTUAL_ENV`, else a non-base `$CONDA_PREFIX`, per `tests/fixtures/venv_forwarding_vectors.tsv`, #729), `json_escape_vectors`, `json_flags_tail`, `load_theme_cache`, `instant_cache_status`, `is_first_instant_cache`, `language_marker_prefilter` (the pre-filter accepts every marker in the agent-exported `__gpy_lang_marker_files`, rejects an empty dir, defers when unset, #785), `oneshot_budget`, `parity`, `prompt_dispatch_positions`, `omitted_segment_positions` (a language cold miss is dropped before positions are assigned, so its neighbour keeps the first/last form), `segment_bg_export`, `missing_core_file_disables_cleanly`, `completions` | none (`integration` runs with the supervisor off and a socket that does not exist) |
| Hostile environment | `hostile_env` (fresh child bash whose rc builds traps and `PROMPT_COMMAND` before sourcing `gpy.bash`; asserts the user's environment survives), `prompt_command_preserves_status` (user `PROMPT_COMMAND` hooks that run after `__gpy_precmd` still see the command's `$?`; system bash 3.2 and the running bash) | none: supervisor off, socket that does not exist |
| IPC edge cases | `ipc_partial_response`, `protocol_version_mismatch`, `ipc_nc_fallback_timeout`, `ipc_timeout_honors_budget` (the socat transport honours `GPY_IPC_TIMEOUT_MS`: a reply at 120 ms is used, no oneshot forked), `register_rejects_error_reply` (an `{"error":…}` reply to register does not count as registered), `workspace_error_keeps_registration` (a workspace error other than "not registered" keeps the registration) | a python3 fake listener |
| Installers, real binaries | `install_e2e_real_binary` (`install.sh`: install, first `fish -i` prompt through config.fish, upgrade with an old `agent.version`, `scripts/uninstall.fish`), `install_oneline_e2e_real_binary` (`install-oneline.sh` for fish, zsh and bash behind a fake `curl`, first prompt on a pty, `scripts/uninstall.*`), `installer_backup_retention` (`install-oneline.sh` and `install.sh` run three times over stub binaries: exactly one `gpy-agent.backup.*` and one `gpy.backup.*` survive and hold the previous run's binary (#807)), `install_oneline_failed_upgrade` (`install-oneline.sh` over a working install with a checksum-valid but non-runnable agent or CLI: the old binaries stay byte-identical, no `.gpy*.new.*` staging file is left, a broken agent fails the install, a broken CLI only warns (#806); a good upgrade still replaces both) | the debug `gpy-agent` and `gpy` (stub binaries for the failed-upgrade test), installed into a sandboxed `~/.local/bin` |
| Installer required files | `install_oneline_required_fish_files` (`install-oneline.sh` for fish behind a fake `curl` that fails one URL: a missing `conf.d/gpy_init.fish` or `functions/fish_prompt.fish` aborts the install with no `gpy-init` block in `config.fish` and no `fish_prompt.fish` symlink, while a missing segment only warns (#808)) | stub `gpy-agent` and `gpy`, installed into a sandboxed `~/.local/bin` |
| Installer rc files | `installer_rc_matrix` (rows of installer x shell x hostile `HOME`/`XDG_CONFIG_HOME` layout, including a space in the path and a symlinked `fish_prompt.fish` that must survive install + uninstall (#744): after install every targeted shell, interactive login and non-login, sources gpy exactly once with empty stderr, and after uninstall every rc file is byte-identical to before; `refuse` rows must touch nothing) | the debug `gpy-agent` and `gpy`, installed into sandboxed homes; each row stops its agent |
| Release payload | `release_smoke` (packages the debug binaries with `scripts/package-release.sh`, runs `scripts/smoke-release.sh`, then proves a broken agent fails it) | the packaged debug `gpy-agent`, installed into a sandboxed `~/.local/bin` |
| Source shape | `ipc_read_preserves_whitespace` (every IPC response read in `zsh/core/ipc.zsh` and `bash/core/ipc.bash` uses `IFS= read -r`, so a bare `read` cannot strip the significant trailing space the character template emits after `❯`, nor mangle the backslashes in the clock's `\D{…}` token), `shell_c_no_interpolation` (no double-quoted `-c` code string for fish/bash/zsh/sh in `tests/` or `scripts/` interpolates a variable, so a checkout path with a space or quote cannot break a suite; pass paths as `$argv`/`$1`, #822) | none |
| Doc tripwire | `bash_limitations_claims` (pins the shell-support matrix rows the E2E tests measure), `ci_gate_shape` (pins the pr-gate/release workflow shape), `cli_reference_claims` (every shipped subcommand is in `docs/user/cli-reference.md`, walked from `--help`), `segment_registry_parity` (the builtin segments, the fish/bash/zsh segment files and the `gpy enable` segment list in `docs/user/cli-reference.md` are the same set, both directions), `test_readme_claims` (the test READMEs name exactly the files that exist) | none |
| Gate and scripts | `toolchain_pin_hooks` (each Rust entry point: `just clippy-strict`, `just lint`, `quality-check.sh --rust-only`/`--fast`/`--fix` fails fast naming the `RUSTUP_TOOLCHAIN` override when the active rustc is not the `rust-toolchain.toml` pin, before any cargo/moon command runs) | none |
| Gate and scripts | `shell_tests_no_agent_leak` (the plain Bash and Zsh `basic`/`parity` suites, run as `run_shell_tests` runs them, leave no `gpy-agent` holding a socket under the scratch root; leaked agents are found with `lsof -U` and stopped by PID) | the debug `gpy-agent`, started by the sourced entry point |
| Gate and scripts | `shell_e2e_stop_agent_term` (`shell_e2e_stop_agent` TERMs a socket holder that `gpy-agent stop` did not release, waits, and only then KILLs: a stub that exits cleanly on TERM must record TERM, one that ignores TERM is still killed, #825) | python3 stub holding an AF_UNIX socket |
| Gate and scripts | `shell_suites_build_agent` (`scripts/test_fish.sh` and `run_shell_tests` build the debug `gpy`/`gpy-agent` unconditionally, not behind an existence check, and honour `CARGO_TARGET_DIR`) | the two scripts, read as text |
| Gate and scripts | `quality_check_fix_cwd` (`quality-check.sh --fix` run from another directory cds to the repo root and does not reformat `.fish` files under the caller's cwd) | none |

### The live-daemon harness: `tests/lib/shell_e2e.sh`

Shared by the Bash and Zsh suites (POSIX sh; Zsh sources it under
`emulate sh`). It provides:

- `shell_e2e_init ROOT` – sandbox `HOME`, `XDG_*`, a socket under the GPY
  test root (the one `scripts/cleanup-test-agents.sh` reaps), the debug agent
  on `PATH` (built if missing), a scratch git repo in `$SHELL_E2E_REPO` with
  one commit and hermetic git config, and an `EXIT` trap that tears it down;
- `shell_e2e_start_agent` / `shell_e2e_stop_agent`;
- `shell_e2e_spawn_client bash|zsh` – an interactive shell on a pty through
  `tests/lib/pty_session.py`, sourcing this checkout's integration; then
  `shell_e2e_send`, `shell_e2e_wait_for REGEX TIMEOUT [OFFSET]`,
  `shell_e2e_size`, `shell_e2e_transcript [OFFSET]` (escapes stripped),
  `shell_e2e_client_pid`, `shell_e2e_assert_registered PID`,
  `shell_e2e_stop_client`, `shell_e2e_dump_transcript`;
- `test_skip "reason"` / `test_require_command NAME` – the skip contract:
  `SKIP:` and exit 0 locally, exit 1 under `CI` (#650). Never `exit 0` on a
  missing prerequisite yourself.

All waits are bounded polls (`shell_e2e_poll SECONDS CMD...`); do not add
fixed sleeps for synchronisation.

## Repository-policy checks

These use bash as a convenient scripting language and assert on the repo
itself (docs, workflows, packaging, dependency claims), not on the shell
integration: `agent_tooling_privacy_claims`, `cleanup_test_agents`,
`contributing_workflow_docs`, `crate_publish_boundary`,
`dependency_advisory_claims`, `doc_links`, `gengo_gix_free`,
`install_checksum_verification`, `install_docs_env_placement`, `install_from_source_docs`,
`install_from_source_docs_isolation`, `install_oneline_verification`, `msrv_consistency`, `nextest_retry_policy`,
`privacy_patterns`, `release_asset_contract`, `release_glibc_floor`,
`release_packaging`, `release_sbom`, `release_version_claims`,
`security_trust_boundary_claims`, `toolchain_pin_scope`,
`windows_shell_support_claims`.

## Running

```bash
just check-shell                                  # everything, as the gate runs it
bash tests/bash/e2e_agent_autostart.test.bash     # one file
GPY_GATE_RELEASE=1 bash tests/bash/install_from_source_docs.test.bash   # builds a release binary
```
