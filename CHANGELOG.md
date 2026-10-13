# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Installer, uninstaller and update scripts print plain status lines without
  emoji or banners.

### Fixed

- The daemon exits with an error if it cannot change into its working
  directory after forking, instead of continuing in the launch directory.

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

[0.1.0]: https://github.com/jpease/gpy/releases/tag/v0.1.0
