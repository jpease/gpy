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
| Live daemon, real terminal | `e2e_agent_autostart`, `e2e_git_live_content`, `e2e_reregister_after_restart`, `e2e_config_sigusr2_reload` | real `gpy-agent`, real `bash -i` on a pty |
| Live daemon, no terminal | `e2e_agent_down_stale_git` (agent down, then up), `path_parity` (`gpy debug paths` vs `__gpy_debug_paths`, cache key against a file the agent wrote) | real `gpy-agent`, functions called directly |
| Function-level | `basic`, `integration`, `debug_trap`, `duration_math`, `json_escape_vectors`, `json_flags_tail`, `load_theme_cache`, `instant_cache_status`, `is_first_instant_cache`, `oneshot_budget`, `parity`, `prompt_dispatch_positions`, `segment_bg_export`, `missing_core_file_disables_cleanly`, `completions` | none (`integration` runs with the supervisor off and a socket that does not exist) |
| IPC edge cases | `ipc_partial_response`, `protocol_version_mismatch`, `ipc_nc_fallback_timeout` | a python3 fake listener |
| Installers, real binaries | `install_e2e_real_binary` (`install.sh`: install, first `fish -i` prompt through config.fish, upgrade with an old `agent.version`, `scripts/uninstall.fish`), `install_oneline_e2e_real_binary` (`install-oneline.sh` for fish, zsh and bash behind a fake `curl`, first prompt on a pty, `scripts/uninstall.*`) | the debug `gpy-agent` and `gpy`, installed into a sandboxed `~/.local/bin` |
| Release payload | `release_smoke` (packages the debug binaries with `scripts/package-release.sh`, runs `scripts/smoke-release.sh`, then proves a broken agent fails it) | the packaged debug `gpy-agent`, installed into a sandboxed `~/.local/bin` |
| Source shape | `ipc_read_preserves_whitespace` (every IPC response read in `zsh/core/ipc.zsh` and `bash/core/ipc.bash` uses `IFS= read -r`, so a bare `read` cannot strip the significant trailing space the character template emits after `❯`, nor mangle the backslashes in the clock's `\D{…}` token) | none |
| Doc tripwire | `bash_limitations_claims` (pins the shell-support matrix rows the E2E tests measure), `ci_gate_shape` (pins the pr-gate/release workflow shape), `cli_reference_claims` (every shipped subcommand is in `docs/user/cli-reference.md`, walked from `--help`), `test_readme_claims` (the test READMEs name exactly the files that exist) | none |

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
`install_checksum_verification`, `install_from_source_docs`,
`install_oneline_verification`, `msrv_consistency`, `nextest_retry_policy`,
`privacy_patterns`, `release_asset_contract`, `release_packaging`,
`release_sbom`, `release_version_claims`, `security_trust_boundary_claims`,
`toolchain_pin_scope`, `windows_shell_support_claims`.

## Running

```bash
just check-shell                                  # everything, as the gate runs it
bash tests/bash/e2e_agent_autostart.test.bash     # one file
GPY_GATE_RELEASE=1 bash tests/bash/install_from_source_docs.test.bash   # builds a release binary
```
