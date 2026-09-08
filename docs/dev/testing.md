# Testing Tiers

What each tier proves, how it proves it, and the one command that runs it.
`./scripts/quality-check.sh` runs every tier marked **gate**; the others run
on demand or in a workflow. Every skip goes through the shared skip contract
(`SKIP: reason`, a pass locally, a failure under `CI`), and the gate's
summary ends with `Skipped: N (names)`.

## The tiers

| Tier | What it proves | How | Run it | In the gate? |
|---|---|---|---|---|
| Rust unit + integration | The agent and CLI logic, IPC, watcher, config, theme export | nextest over the library and the `gpy-agent/tests/*.rs` targets | `(cd gpy-agent && cargo nextest run --features test-support)` | gate |
| CLI lifecycle and surface | `gpy start/stop/restart/status` and `gpy doctor` against a live and a stopped agent; error messages; palette; `--help` snapshots; `init` and the wizard on a pty | Real binaries in an isolated HOME/XDG sandbox (`CliTestEnv`), a real agent, `portable-pty` for the interactive flows, insta snapshots for `--help` | `(cd gpy-agent && cargo nextest run --features test-support --test agent_lifecycle_cli_tests --test cli_error_messages_tests --test palette_cli_e2e_tests --test cli_help_snapshot_tests --test init_pty_tests --test wizard_pty_tests)` | gate |
| Fish prompt content | The rendered `fish_prompt` carries the exact git (branch, ahead, dirty, untracked, stash, detached, rebase), language and directory tokens, plus the frame | A real repository, a real agent, escapes stripped, content asserted (never mere presence) | `fish tests/fish/e2e_prompt_content.test.fish` | gate |
| Fish interactive sessions | An idle prompt repaints with no keystroke after a file change; two shells share one agent; a `config.toml` edit reaches a Fish client via SIGUSR2 | A real `fish -i` on a pseudo-terminal driven by `tests/lib/pty_session.py` | `fish tests/fish/e2e_interactive_session.test.fish` | gate |
| Bash/Zsh live daemon | Autostart to first prompt; live git content (Zsh repaints idle, Bash at the next prompt, by measurement); re-registration after an agent restart; stale git recovery with the agent down; SIGUSR2 config reload; nc fallback bounded; path parity with `gpy debug paths` | `tests/lib/shell_e2e.sh`: a sandbox, a real agent, a real `bash -i` / `zsh -i` on a pty | `bash tests/bash/e2e_*.test.bash` and `zsh tests/zsh/e2e_*.test.zsh` (one file at a time) | gate |
| Installer, real binaries | `install.sh`: install, first `fish -i` prompt through config.fish, upgrade with an old `agent.version` (config preserved, binary backed up, agent replaced), `scripts/uninstall.fish` leaving nothing behind. `install-oneline.sh`: the same for fish, zsh and bash behind a fake `curl` | The debug binaries packaged like the release archive, installed into a sandboxed `~/.local/bin` | `bash tests/bash/install_e2e_real_binary.test.bash` and `bash tests/bash/install_oneline_e2e_real_binary.test.bash` | gate |
| Release smoke | The packaged binaries install, report the tag's version, render a oneshot, answer `status`, and print one prompt per shell; a binary that answers `--version` but nothing else fails it | `scripts/smoke-release.sh` on the payload `scripts/package-release.sh` produced; the `smoke` job in `release.yml` blocks `create-release` | `bash tests/bash/release_smoke.test.bash` locally; `scripts/smoke-release.sh --package dist --version vX.Y.Z` on a payload | gate (the local test) |
| Container fresh install | A stock `ubuntu:24.04` image with an empty `$HOME` installs from an archive built from the checkout and renders one prompt in fish, zsh and bash | `tests/docker/Dockerfile.fresh-install` (release build inside the image), then `smoke-release.sh` as an unprivileged user; the `fresh-install` job in `pr-gate.yml` | `scripts/test-fresh-install.sh` (needs a docker daemon) | workflow |
| Fish, Bash, Zsh function-level suites | Segment rendering, IPC encoding, cache keys, completions, supervisor cadence, and the policy tripwires (documented claims, retry policy, CI gate shape) | Sourced functions, stub listeners, text pins | `./scripts/test_fish.sh`; `./scripts/quality-check.sh --shell-only` | gate |
| Windows | The CLI-only claim: build, library unit tests, and the five hermetic CLI integration targets | `.github/workflows/windows-gate.yml`, called by `pr-gate.yml` and `cross-platform-test.yml`; compile reproduced locally in a Linux container with mingw | `gh workflow run pr-gate.yml --ref main -f run_windows=true` | workflow (manual) |
| Coverage | A line-coverage figure for `gpy-agent` | `cargo llvm-cov nextest` over the same suite as the Rust tier; non-gating; `lcov.info` uploaded by the ubuntu pr-gate | `just coverage` | workflow (non-gating) |

The Fish, Bash and Zsh suite READMEs (`tests/fish/README.md`,
`tests/bash/README.md`, `tests/zsh/README.md`) list every file in each suite;
`gpy-agent/tests/README.md` covers the Rust harnesses.

## Rules every tier follows

- Assert content, not presence: strip escapes first, then look for the exact
  token (glyphs are looked up in `git_resolver.rs`, never typed).
- No fixed sleeps for synchronisation: bounded polls, and the transcript or
  agent log dumped on failure.
- No test mutates the checkout; every spawned agent has a cleanup guard under
  the GPY-owned test root that `scripts/cleanup-test-agents.sh` reaps.
- A new test is run once against a deliberately broken tree before it lands,
  and the commit says so.

## Coverage

`just coverage` writes `gpy-agent/lcov.info` and prints the summary table.
The ubuntu pr-gate runs the same command as a non-gating step and uploads the
file as the `coverage-lcov` artifact. There is no threshold yet.

First recorded figure (2026-09-07, macOS, `cargo llvm-cov nextest
--features test-support` over the full Rust suite): see the table below,
kept in step with the number in issue #654.

| Date | Lines | Functions | Regions | Notes |
|---|---|---|---|---|
| 2026-09-07 | 91.92% (37402 lines, 3021 missed) | 89.59% (4208 functions, 438 missed) | 91.96% | first measurement, local macOS run, 2109 tests |
