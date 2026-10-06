# Fish Shell Integration Tests

This directory contains integration tests for the Fish shell prompt and GPY initialization.

## Test Files

### Core Integration Tests

- **`init_integration.test.fish`** - Tests the core initialization flow
  - Sources `core/init.fish` and validates all segments are properly registered
  - Verifies all enabled segments have both `detect` and `render` functions
  - Confirms `fish_prompt` returns non-empty output with all segments
  - Tests theme variables, icons, and utility functions
  - 12 comprehensive tests

- **`fresh_install.test.fish`** - Sources `fish/core/init.fish` from the checkout with no existing config
  - Scrubbed `$HOME` and `$XDG_CONFIG_HOME`; the agent enabled, the supervisor off
  - Verifies the defaults load: segments registered, icons set, `fish_prompt` renders in git and non-git directories
  - Runs no installer. The installers themselves are exercised end to end, with the real binaries, by `tests/bash/install_e2e_real_binary.test.bash` (`install.sh`: install, first prompt, upgrade, uninstall) and `tests/bash/install_oneline_e2e_real_binary.test.bash` (`install-oneline.sh` for fish, zsh and bash) (#649)
  - 10 tests

### Installer Tests

- **`install_dev_remove_conflicting_binaries.test.fish`** - `install-dev.fish` removes only a stale copy of our own `gpy`/`gpy-agent` under `$HOME` that shadows the install dir on PATH; an unrelated tool sharing the name, a tool-manager shim, anything after the install dir, a shadowing copy outside `$HOME` (warned about), and every copy when the install dir is off PATH are kept (#649, #693)
- **`install_dev_stop_running_agent.test.fish`** - `install-dev.fish` stops only the agent bound to its own socket, never by process name
- **`uninstall_rc_marker_roundtrip.test.fish`** - install then uninstall restores `config.fish` / `.zshrc` / `.bashrc` byte-identically, with stub binaries in a temp package (never the checkout's `bin/`)
- **`uninstall_no_repo_checkout.test.fish`** - the uninstallers run standalone without a clone

### End-to-End Tests

- **`e2e_prompt_content.test.fish`** - Renders `fish_prompt` through a real agent from a real repository and asserts the exact git (branch, ahead, dirty, untracked, stash, detached, rebase), language (names, venv version) and directory (every display mode) tokens, plus the prompt frame (#644)
- **`e2e_interactive_session.test.fish`** - Drives real interactive `fish -i` sessions on a pseudo-terminal through `tests/lib/pty_session.py`: a tracked-file edit repaints the idle prompt with no keystroke, two shells share one agent and both repaint after a commit typed in one of them, a `config.toml` edit reaches a Fish client via the `.reload` doorbell (theme switch, new segment, icons off) without a keypress (#645), and an `exec fish` survives the agent's notifications during its startup (#674)
- **`e2e_agent_autostart.test.fish`** - Tests agent auto-start behavior
- **`e2e_live_updates_signal.test.fish`** - Tests live prompt updates via the SIGURG doorbell
- **`doorbell_signal.test.fish`** - Tests the SIGURG doorbell handler: repaint on every ring, `.reload`/`.reregister` flags consumed once, untrack removes them (#674)

### Feature-Specific Tests

- **`segment_toggle.test.fish`** - Tests toggling segments on/off
- **`segments.test.fish`** - Tests individual segment functionality
- **`theme_prompt_render.test.fish`** - Tests theme rendering
- **`ipc_security.test.fish`** - Tests IPC security features
- **`ipc_nc_fallback_timeout.test.fish`** - Tests the `nc` fallback bounds its wait against a wedged agent (#299)
- **`maybe_refresh_nonblocking.test.fish`** - Background git/language refreshes return at once against a slow agent, and with the agent down never fork `gpy-agent oneshot` or hide the directory segment (#685)
- **`workspace_error_keeps_registration.test.fish`** - A workspace error reply (e.g. `cd /etc` rejected) keeps the registration; only "not registered" re-registers (#764)
- **`main.test.fish`** - Basic smoke tests

### Every file in this directory

`./scripts/test_fish.sh` auto-discovers `tests/fish/*.test.fish`, so this list
is the whole suite; `tests/bash/test_readme_claims.test.bash` fails when a
file is added without a line here or a line names a file that is gone. The
sections above describe the ones worth reading first.

**Live daemon end-to-end (`e2e_*`)**

- `e2e_agent_autostart.test.fish`
- `e2e_agent_down_oneshot_fallback.test.fish`
- `e2e_existing_client_reregisters_after_restart.test.fish`
- `e2e_git_cold_miss_bounded_sync.test.fish`
- `e2e_git_live_content.test.fish`
- `e2e_git_show_upstream_reload.test.fish`
- `e2e_git_variant_fallback_bounded_sync.test.fish`
- `e2e_interactive_session.test.fish`
- `e2e_lang_variant_fallback_bounded_refresh.test.fish`
- `e2e_live_updates_signal.test.fish`
- `e2e_prompt_content.test.fish`

**Hostile environment**

- `hostile_env.test.fish`

**Installers, uninstallers and release plumbing**

- `install_completions_wiring.test.fish`
- `install_dev_config_theme_preserve.test.fish`
- `install_dev_remove_conflicting_binaries.test.fish`
- `install_dev_stop_running_agent.test.fish`
- `install_oneline_cli_binary.test.fish`
- `install_oneline_file_lists.test.fish`
- `install_oneline_main_wrapper.test.fish`
- `install_sh_cli_binary.test.fish`
- `release_workflow_cli_assets.test.fish`
- `uninstall_no_repo_checkout.test.fish`
- `uninstall_rc_marker_roundtrip.test.fish`
- `update_config_theme_preserve.test.fish`

**IPC, protocol and agent lifecycle**

- `agent_circuit_breaker_expiry.test.fish`
- `agent_restart_quiet_when_socket_missing.test.fish`
- `cache_key_vectors.test.fish`
- `doorbell_signal.test.fish`
- `ipc_nc_fallback_timeout.test.fish`
- `ipc_partial_response.test.fish`
- `ipc_request_status.test.fish`
- `ipc_security.test.fish`
- `json_escape_vectors.test.fish`
- `json_extract_and_protocol_stdout.test.fish`
- `json_flags_tail.test.fish`
- `maybe_refresh_nonblocking.test.fish`
- `path_parity.test.fish`
- `policy_helpers.test.fish`
- `prompt_autostart_backoff.test.fish`
- `protocol_version_mismatch.test.fish`
- `supervisor_cadence.test.fish`
- `test_helpers_scoped_kill.test.fish`
- `workspace_error_keeps_registration.test.fish`

**Instant-prompt and theme caches**

- `character_directory_memoization.test.fish`
- `completions_dynamic_cache.test.fish`
- `config_hot_reload.test.fish`
- `instant_cache_byte_exact.test.fish`
- `instant_cache_status.test.fish`
- `instant_cache_ttl.test.fish`
- `is_first_instant_cache.test.fish`
- `language_cache_nongit.test.fish`
- `language_cache_stale_refresh.test.fish`
- `theme_export_cache.test.fish`
- `theme_reload_clears_optional_vars.test.fish`

**Segments and rendering**

- `character_status_suppression.test.fish`
- `clock_padding.test.fish`
- `completions_dynamic.test.fish`
- `directory_truncation_export.test.fish`
- `disabled_footprint_prompt.test.fish`
- `duration_threshold.test.fish`
- `fish_segment_parsing.test.fish`
- `fresh_install.test.fish`
- `hostname_segment.test.fish`
- `init_integration.test.fish`
- `language_segment_detect_subdir.test.fish`
- `language_venv.test.fish`
- `main.test.fish`
- `prompt_dispatch_positions.test.fish`
- `renderer_transparent_bg.test.fish`
- `segment_bg_tracking_empty_response.test.fish`
- `segment_lazy_reload.test.fish`
- `segment_toggle.test.fish`
- `segments.test.fish`
- `starship_parity.test.fish`
- `starship_preset_import_parity.test.fish`
- `status_segment.test.fish`
- `theme_prompt_render.test.fish`
- `username_segment.test.fish`

### Driving a real terminal: `tests/lib/pty_session.py`

`tests/lib/pty_session.py` (standard-library Python 3, no third-party modules)
spawns a program in a pseudo-terminal and lets a shell test drive it. A session
lives in a directory holding a `transcript` (everything the program wrote),
a `control` FIFO (bytes written there go to the program's stdin), the driver
`pid`, and an `exit` file once the program ends.

```fish
python3 tests/lib/pty_session.py start $dir -- fish --no-config -i -C 'source fish/core/init.fish'
python3 tests/lib/pty_session.py wait-for $dir '❯' 10          # regex, timeout; prints the transcript length
set -l offset (python3 tests/lib/pty_session.py size $dir)      # "anything new after now"
python3 tests/lib/pty_session.py send $dir 'git status
'      #

python3 tests/lib/pty_session.py send $dir 'git status\r'      # \r \n \t \e \x04 escapes decoded
python3 tests/lib/pty_session.py wait-for $dir 'main' 5 $offset
python3 tests/lib/pty_session.py stop $dir
```

`wait-for` only reads the transcript, so a test can watch for output that
arrives with no keystroke sent (the live-repaint case). The driver answers the
terminal queries fish 4 sends at startup and around redraws (Primary Device
Attributes, cursor position), which a plain pipe never would; without those
replies fish renders no prompt. Tests that need it skip through
`test_skip` when `python3` is missing (exit 0 locally, exit 1 under `CI`).

### No manual tier

There is no `tests/manual/` any more (#655). Everything the old scripts
exercised by hand now gates: the "no JSON leaks into the prompt" check in
`e2e_prompt_content.test.fish` (#644), the live-update walkthrough in
`e2e_interactive_session.test.fish` (#645), the socket → cache → oneshot flow
in `e2e_agent_down_oneshot_fallback.test.fish`, and doorbell repaints in
`doorbell_signal.test.fish`. For an interactive look at a prompt, run a real
`fish -i` in the sandbox `tests/lib/pty_session.py` builds, the way the E2E
tests do.

## Running Tests

### Run All Fish Tests

```bash
./scripts/test_fish.sh
```

This script automatically discovers and runs all `*.test.fish` files in this directory.

### Run Individual Tests

```bash
fish tests/fish/init_integration.test.fish
fish tests/fish/fresh_install.test.fish
```

Tests work from any directory - they automatically find the repository root.

### Syntax Check

```bash
fish -n tests/fish/*.test.fish
```

## CI Integration

The Fish suite runs in three places, all through `./scripts/test_fish.sh`
(syntax checks with `fish -n`, formatting with `fish_indent --check`, then
every `*.test.fish` with `Skipped:`/`Retried:` tallies at the end):

1. **Locally, on every push**: the `pre-push` hook's `shell-gate` (`just
   check-shell`), which is the only enforced gate while the workflows'
   automatic triggers are off (#556).
2. **`.github/workflows/pr-gate.yml`**, `Shell integration gate` step on the
   ubuntu job, when dispatched by hand (`gh workflow run pr-gate.yml --ref
   main`); it installs Fish 4, socat, python3, starship and the other
   prerequisites, and under `CI` a skipped test fails (#650).
3. **`.github/workflows/release.yml`**, `validate` job, on every tag.

There is no `.github/workflows/test.yml`.

## Writing New Tests

### Test Template

```fish
#!/usr/bin/env fish
# tests/fish/my_test.test.fish

set -g test_failures 0

function test_fail
    set -g test_failures (math $test_failures + 1)
    echo "❌ FAIL: $argv[1]"
end

function test_pass
    echo "✅ PASS: $argv[1]"
end

# Find repo root
set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

# Your tests here
if some_condition
    test_pass "Test description"
else
    test_fail "Test description"
end

# Exit with failure if any tests failed
if test $test_failures -gt 0
    exit 1
end
```

### Best Practices

1. **Always find repo root** - Use the pattern above to ensure tests work from any directory
2. **Disable the supervisor for unit tests** - Set `GPY_AGENT_SUPERVISOR_ENABLED=0` (keep `GPY_AGENT_ENABLED=1`) to avoid spawning a persistent daemon while still exercising the full init path. Setting **both** to `0` trips init.fish's "completely disabled" early-exit, which skips icon initialization and segment loading — only do that when you are specifically testing the disabled-footprint mode
3. **Clean up temporary files** - Use `mktemp -d` and `rm -rf` in cleanup
4. **Test both git and non-git directories** - Ensure prompt works in all environments
5. **Provide clear test output** - Use descriptive test names and echo progress
6. **Exit with proper status** - Return 0 for success, 1 for failure

## Integration with Rust Tests

The Rust test suite (in `gpy-agent/tests/`) includes complementary Fish integration tests:

- **`fish_integration_tests.rs`** - End-to-end tests that spawn Fish processes
  - Uses `env!("CARGO_BIN_EXE_gpy-agent")` to test against build artifacts
  - Tests agent auto-start behavior
  - Tests prompt works without agent (degraded mode)
  - Tests rapid Fish restarts don't crash the agent

These Rust tests verify the agent's behavior when integrated with Fish, while the Fish tests in this directory verify the Fish prompt code itself.
