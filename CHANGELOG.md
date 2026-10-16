# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `gpy setup --redetect-icons` re-runs Nerd Font detection and rewrites only `ui.show_icons`, in place. A first-time `gpy setup` still runs detection without overwriting an existing value.

### Changed

- **Breaking:** `gpy-agent` keeps only `start`, `stop`, `status`, `oneshot`, `theme export` and `config reload`. Removed, with their replacements: `gpy-agent init` → `gpy setup` (`init --force` → `gpy setup --redetect-icons`); `gpy-agent theme validate`/`theme import` → `gpy theme validate`/`gpy theme import`; `gpy-agent palette …` → `gpy palette …`; `gpy-agent doctor` → `gpy doctor`; `gpy-agent config get`/`config list` → `gpy config get`/`gpy config show`.
- `gpy` output uses colored ✓/✗/! only for pass/fail/warn results; single-action confirmations are plain text. When stdout is not a terminal, `NO_COLOR` is set, `TERM=dumb`, or `ui.show_icons = false`, these print as `ok:`/`FAIL:`/`warn:`. `gpy segments` marks enabled segments with `[x]`.
- **Breaking:** the repaint throttle environment variable `GPY_SIGUSR1_THROTTLE_MS` is now `GPY_SIGURG_THROTTLE_MS`. The old name is no longer read.
- **Breaking:** the one-line installer is a release asset, not a file on `main`: use `curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh`, or `releases/download/vX.Y.Z/install-oneline.sh` for a specific release. The published copy has its release tag built in and installs exactly that release. The old `raw.githubusercontent.com/.../main/install-oneline.sh` URL is no longer documented.
- `install-oneline.sh` installs the Fish, Zsh and Bash scripts from one archive, `gpy-shell-files.tar.gz`, after checking its published SHA-256, instead of downloading each script from a branch or tag unverified. `GPY_VERSION` can therefore only name a release that publishes that archive; releases up to v0.3.1 do not.
- Releases also publish `install-oneline.sh`, `uninstall.sh` and `uninstall.fish` (checksummed and attested like every other asset); the installer's uninstall hint points at them.
- `SECURITY.md` states the trust model (see [Trust model](SECURITY.md#trust-model)); `gh attestation verify` is the optional independent check.
- CI installs starship from a pinned release tarball checked against a committed SHA-256, and rustup-init in the WSL smoke job the same way, instead of piping install scripts into a shell.
- **Breaking (library API):** `EndpointHandle::builder` now takes an `EndpointDeps` (new public struct in `gpy_agent::ipc::server`) holding the required collaborators, so a missing one is a compile error. The runtime `build()` errors for missing fields are gone; the socket path, watcher, guard settings and palette cache stay optional setters.

## [0.3.1] - 2026-10-15

v0.3.0 was tagged but never published: its Windows build failed. This release
carries its changes.

### Added

- `gpy setup`: installs GPY's scripts for Fish, Zsh or Bash and wires the rc
  file, with the same layout and `gpy-init` markers as the curl installer. It
  makes Homebrew and source builds work for every shell, not only Fish.
- A GPY vs Starship performance table in the README and a page that gives the
  versions, machine and method behind it and how to run the comparison
  ([docs/user/performance-vs-starship.md](docs/user/performance-vs-starship.md)).

### Changed

- `install-oneline.sh` installs the shell files and wires the rc file by running
  `gpy setup`, so every install channel shares one implementation. The `gpy`
  CLI is now required: a missing, unverifiable or non-functional CLI aborts the
  install instead of warning. Pinning `GPY_VERSION` to a release older than the
  one that adds `gpy setup` no longer works.
- `install.sh` (the release-archive installer) now sets up Fish, Zsh or Bash,
  by running `gpy setup` against the archive's own `fish/`, `zsh/` and `bash/`
  trees; it was Fish-only. The shell comes from `$SHELL` (`GPY_SHELL`
  overrides). The `gpy` CLI is required, as in the one-line installer.
- The Homebrew formula ships the Fish, Zsh and Bash trees, no longer depends on
  Fish, and finishes with `gpy setup` instead of a Fish-only caveat.
- `just bench-compare` pins GPY v0.2.0.

## [0.2.0] - 2026-10-14

### Added

- Transient prompt in Fish 4.2+ and Zsh (`ui.transient_prompt`, on by
  default): after Enter, the prompt collapses to the clock's time, the prompt
  symbol and the command. With it off, the full prompt is redrawn instead.
  Either way the clock in the scrollback shows when Enter was pressed. Bash
  keeps the full prompt and the time it was drawn.
- `just bench-compare`: reproducible GPY vs Starship benchmark on repositories
  pinned to exact commits, with pinned or latest releases of both tools and a
  CPU, swap and thermal monitor that re-runs any timed block measured on a busy
  machine (`benchmarks/compare-starship/`). The monitor subtracts the CPU the
  tools themselves used, so a multi-core `git status` on a large repository is
  not mistaken for other load. `--untracked-cache unset|on|off` chooses how the
  fixtures set `core.untrackedCache` (default `unset`, a typical machine).

### Changed

- Installer, uninstaller and update scripts print plain status lines without
  emoji or banners.
- Dependencies updated to their latest releases, including `tokio` 1.53.3.

### Fixed

- The daemon exits with an error if it cannot change into its working
  directory after forking, instead of continuing in the launch directory.
- Bash reads a `GPY_IPC_TIMEOUT_MS` with a leading zero (`0150`) as decimal,
  as Fish and Zsh do, instead of as octal (or failing on `08`/`09`).
- A relative `HOME` is ignored by the agent and CLI, as it already was by the
  shells, so both resolve the same socket (`/tmp/gpy/gpy.sock`) instead of a
  path under the agent's working directory.

Schema compatibility: `ui.transient_prompt` is a new optional config field with
a serde default, so existing `config.toml` files load unchanged (MINOR). See
`gpy-agent/SCHEMA_EVOLUTION.md`.

## [0.1.0] - 2026-10-10

First public release.

### Added

- **Agent.** `gpy-agent`, a Rust daemon that computes git status and language
  information off the prompt path, caches it, and serves it to shells over a
  Unix domain socket. Shells fall back to a one-shot render when no agent is
  reachable, and to an on-disk instant-prompt cache for the first render.
- **Shells.** Fish 3.6+, Zsh 5.8+ and Bash 3.2+ (5.0+ recommended) on Linux
  and macOS, and on Windows under WSL. Native Windows is CLI-only. See
  [Supported Platforms and Shells](docs/INSTALL.md#supported-platforms-and-shells).
- **Live updates.** A filesystem watcher (FSEvents/inotify, with a polling
  fallback and a periodic reconcile) notifies registered shells with a
  SIGURG doorbell. Fish and Zsh repaint an idle prompt in place; Bash shows
  the change at the next prompt. Theme and config edits hot-reload.
- **Segments.** `directory`, `git`, `language`, `duration`, `clock` and
  `status`, plus opt-in `hostname` (SSH-only by default) and `username`.
  - Git: branch, ahead/behind, staged/unstaged/untracked/conflict counts,
    detached HEAD, in-progress operation with rebase progress
    (`↻ REBASING 3/5`), and stash count (`git.stash_enabled`).
  - Language: detection from GitHub Linguist's tables (`gengo-language`) over a
    bounded directory walk, with tool versions; Python reports the active or
    project-local virtualenv's interpreter.
- **Themes and palettes.** Starship-style `format` templates rendered by the
  agent. Builtin themes `default`, `starship` and `text`; builtin palettes
  `default`, `starship`, Nord, Gruvbox dark (medium) and the four Catppuccin
  flavours. Per-state and per-language style overrides.
- **Starship migration.** `gpy theme import <starship.toml>` converts a
  Starship config into a GPY theme and palette, warning about anything it
  cannot map. `gpy palette import` reads base16/base24 scheme files.
- **CLI.** `gpy` with `start`/`stop`/`restart`/`status`, `doctor`, `config`
  (including an interactive TUI wizard), `theme`, `palette`, `segments`,
  `plugin validate`, `debug paths`, and shell completions for all three
  shells.
- **Plugins.** Segment plugins loaded at runtime from a manifest; see
  [docs/dev/plugins.md](docs/dev/plugins.md).
- **Installation.** One-line installer, release-archive installer, a Homebrew
  tap, a Fisher plugin, and uninstall scripts for Fish and POSIX shells.

### Security

- The socket is created with `0600` permissions; registration validates PIDs with `kill(pid, 0)`, so the agent
  only signals the caller's own processes.
- IPC messages are validated (path length, NUL bytes, control characters) and
  capped at 64 KiB. Repaint signals to a shell are throttled (150 ms default,
  `GPY_SIGUSR1_THROTTLE_MS`).
- Bash and Zsh receive segment text escaped for their prompt syntax
  (`bash-prompt` and `zsh-prompt` formats), so a directory or branch name such
  as `$(cmd)` is displayed, never executed.
- Release downloads are verified: every release publishes SHA-256 sidecars, a
  `SHA256SUMS` manifest, a CycloneDX SBOM and GitHub build-provenance
  attestations. The installers refuse a binary whose checksum is missing or
  wrong, and remove only `com.apple.quarantine`, after verification.
- GitHub Actions workflows run with read-only default permissions and
  SHA-pinned actions. See [SECURITY.md](SECURITY.md).

### Known limitations

- Binaries are not code-signed or notarized ([SECURITY.md](SECURITY.md#code-signing-not-done-deliberately)).
- Bash cannot repaint an idle prompt, and Bash 3.2 has no duration segment
  ([Bash Limitations](docs/user/bash-limitations.md)).
- No transient prompt in any shell.

[0.3.1]: https://github.com/jpease/gpy/releases/tag/v0.3.1
[0.2.0]: https://github.com/jpease/gpy/releases/tag/v0.2.0
[0.1.0]: https://github.com/jpease/gpy/releases/tag/v0.1.0
