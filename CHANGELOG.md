# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Rendering the language segment no longer runs a project's `.venv`/`venv`
  Python interpreter to read its version. Only a virtualenv the shell
  activated is probed; a project venv's version comes from `pyvenv.cfg`, or is
  omitted.
- `gpy-agent start` refuses a symlinked or foreign-owned socket directory,
  for both the default runtime directory and `GPY_AGENT_SOCKET_PATH`/`--socket`.
  A group/world-writable directory is tightened to `0700` if it is GPY's
  default runtime directory, and refused unchanged if you chose it.
- Theme format templates can no longer emit terminal escape sequences:
  `gpy theme validate` rejects control characters other than newline, and
  rendering strips them from literal text.
- FIFOs and other special files in a project no longer hang language
  detection or version reading; they are refused when opened, not read.
- Instant-prompt cache files and `theme-export.{fish,bash,zsh}` are now
  created owner-only (`0600`), in a `0700` `instant-prompts` directory.
- Bash no longer `eval`s an unvalidated username when `$HOME` is empty.

### Changed

- Language markers under gitignored directories (e.g.
  `node_modules/x/package.json`) no longer trigger language re-detection.
- `gpy doctor` reports unknown config keys and the file they are in.
- A socket directory set through `GPY_AGENT_SOCKET_PATH` must be private: a
  socket placed directly in a shared directory such as `/tmp` is refused.
- Theme colors are stored in one canonical spelling (`bright-magenta` becomes
  `brmagenta`).
- Template colors follow the active palette under every spelling: `purple`,
  `gray`/`grey`, `brblack` and `bright-purple` take the palette's
  `magenta`/`bright_black`/`bright_magenta` entry. A key spelled exactly as
  written still wins, and a palette defining several keys for one color loads
  with a warning from `gpy palette validate` and `gpy doctor`.
- Theme `[segments.<name>]` sections and plugin segment names must be valid
  segment names, and two names that would share shell variables (`a-b` and
  `a_b`) are rejected with an error naming both.
- `ui.enabled_segments` entries are trimmed and lowercased when the config is
  loaded or saved.
- `ui.enabled_segments` keeps only the first occurrence of a segment, in any
  spelling, so a repeated entry renders once.
- `git.skip_paths` always applies to whole repositories: a repository is
  skipped when its root is at or under an entry, for prompt requests and
  background refreshes alike. An entry inside a repository no longer hides git
  only for prompts in that subdirectory.
- `git.timeout_seconds` is the budget for a whole git status: every git
  command it runs shares it, so a blocking file under `.git` costs one timeout
  instead of one per command.
- Bash and Zsh: throttled background-refresh checks and cache-hit renders no
  longer fork subshells; Zsh forks nothing, Bash only for `realpath` and
  `stat`.
- The toolchain pin is Rust 1.99.1 (the minimum supported version stays 1.99).
- The agent's file-watcher threads no longer wake on a timer while idle: they
  block until a filesystem event arrives or a debounce deadline falls due,
  removing about 150 wakeups per second from an idle agent.
- `gpy-agent theme export --format bash`/`zsh` no longer emits
  `__gpy_plugin_segment_file_*` variables; plugin segment files are Fish-only.
- Bash resolves the working directory once per `cd` instead of forking
  `realpath` on every prompt. A directory re-pointed by a symlink change
  without a `cd` keeps its old resolution until the next `cd`, as in Fish.
- Bash and Zsh build agent requests without forking a subshell for each
  JSON-escaped value.
- `{"op":"status"}` now replies through the JSON formatter, as
  `{"AgentStatus":{...}}` in every format; the keys are unchanged and now
  appear in alphabetical order.
- **Breaking:** the agent accepts only `{"op":...}` IPC messages. The serde
  externally-tagged form (`{"RepositoryStatus":{...}}`, `"Ping"`) is rejected,
  and `schemas/message.json` no longer lists it. The shell integrations
  already send only the `op` form.
- **Breaking (library):** `gpy_agent::ipc::protocol::serialize_message` is
  removed, and `Message`, `SafePath` and `ClientPid` no longer implement
  `Serialize`/`Deserialize`.
- The library's `GuardSettings` no longer has a `request_timeout` field; it
  was never read. The IPC request timeout is still `agent.timeout_seconds`.

### Removed

- The hidden, deprecated `gpy theme use --apply-layout` alias; use `--force`.
  `gpy theme import --apply-layout` is unchanged.
- **Breaking:** `gpy-agent oneshot --format` and the IPC `format` field accept only `json`, `ansi`, `bash-prompt` and `zsh-prompt`. The unused `fish`, `fish-source` and `zsh` formats and the never-implemented `bash-source` and `zsh-source` are gone; no shipped shell sent them.
- **Breaking:** the unused IPC operations `theme` and `latency_stats` are removed; no shipped shell or CLI sent them.
- `just install` (`install-dev.fish`) installs Fish through `gpy setup`:
  `gpy_init.fish` is sourced once and built-in themes are no longer copied.
  Older dev installs may have `default.toml`, `starship.toml` and `text.toml`
  in `~/.config/gpy/themes` shadowing the built-ins; delete them.
- Bash: theme variables (`__gpy_*`, `__color_*`, `__icon_*`, `__prompt_*`,
  `__enabled_segments`) are plain shell variables, no longer exported to every
  child process. `GPY_*` settings are still exported.
- `gpy start`, `stop`, `restart`, `status` and `doctor` run the `gpy-agent`
  installed beside `gpy`, falling back to `PATH` only when there is none, so an
  older `gpy-agent` earlier on `PATH` is no longer used. `gpy doctor` shows
  which agent binary it checked.

### Removed

- `scripts/update.fish`, which never detected a version change; run
  `just install` from the checkout.
- `uninstall.fish` and the `fisher-gpy.tar.gz` release asset are gone. Use
  `sh uninstall.sh`, which removes the Fish integration too (rc block, prompt
  symlink, prompt restore). GPY was never installable through Fisher: the
  plugin tree shipped no agent binary.
- Fish: the `gpy_lang_status` debug function, which printed settings nothing
  reads and called a function that no longer exists.

### Fixed

- `SECURITY.md` now lists 0.4.x, the current release series, as supported.
- Fish: `prompt_debug vars` reports the duration threshold the duration
  segment uses (`__duration_threshold_ms`).

- A slow git scan can no longer overwrite a newer cached status.

- Idle prompts no longer repaint spuriously after the instant-prompt write
  cache overflows.

- `config/config.example.toml` documents `git.max_ahead_behind` and
  `git.watch_worktree`.

- Bash and Zsh keep a newline in a segment `format` when the agent answers
  live, instead of cutting the segment at it. Agent replies now carry an
  embedded newline as the RS byte (`\x1e`) so each reply stays one line;
  an older shell shows that byte instead of the line break until it reloads.
- Colors the shells draw themselves (clock, hostname, status, delimiters)
  follow the active palette's value for an ANSI name such as `red`, as
  agent-rendered segments already did. An ANSI index (`1`) still uses the
  terminal's own color.
- The uninstallers remove `fish_prompt.fish` only when it is GPY's own prompt,
  not whenever the file mentions gpy.
- `gpy setup --shell bash` adds the login-shell loader even when the login
  file mentions `.bashrc` only in a comment.
- `gpy doctor` fails when the `gpy-agent` it finds exits with an error, and
  shows the exit status and the error output.
- Language and git segments appear in projects whose encoded cache key is
  longer than 200 characters, instead of staying hidden.
- Bash and Zsh background-refresh throttling works for long or non-ASCII
  project paths, with no "file name too long" errors.
- On Bash 4.2+, the per-prompt agent health check no longer launches `date`.
- A palette file that fails to load no longer blanks prompt segments on
  reload; the agent keeps the last good palette until the file is fixed.
- Outside Git, a running agent notices project markers added or removed above
  the current directory within the language refresh interval.
- `git.skip_paths` entries created or retargeted (symlinks) while the agent
  runs take effect within seconds, without a restart.
- Plugin segments whose names repeat separators or end in `_` (`my--seg`,
  `seg_`) load in Fish.
- Theme colors accept exactly the spellings format styles accept
  (`bright-magenta`, `brpurple`) and reject malformed values such as
  `##112233`, which used to render an empty segment.
- A config spelling `"Directory"` or `" git "` in `ui.enabled_segments` renders
  those segments.
- Adding or removing a user theme that overrides the active plugin or builtin
  theme takes effect in the running agent, matching a fresh render.
- The agent's periodic reconcile no longer scans or writes prompt caches for
  repositories where git is disabled or under `git.skip_paths`.
- Changing `agent.supervisor` settings reaches already-open shells at their
  next prompt.
- `gpy-agent config reload` prints the agent's reason and exits 1 when the
  agent rejects the new config (it keeps the previous one) or does not answer.
- `gpy-agent start` prints the daemon's own startup error, such as an
  uncreatable cache directory and why, instead of a generic "exited before it
  was ready".
- `install-dev.fish` no longer deletes a custom `fish_prompt.fish` that merely
  mentions GPY; it backs it up and replaces only GPY's own prompt.
- Disabling git or adding a `git.skip_paths` entry, then reloading, removes
  that repository's cached git prompts and repaints open shells.
- Fish background refreshes for paths that differ only in punctuation no
  longer share one throttle.
- Fish draws a newline inside an agent-rendered segment format as a line break
  on the oneshot and cached paths, as Bash and Zsh do.
- Fish-drawn segments (clock, hostname, status, delimiters) show ANSI index
  colors and the `orange`/`brown` palette roles; Bash and Zsh shell-drawn
  segments resolve those roles too.
- An error creating the runtime directory names the directory and the cause.
- A theme file edit reloads once, within about a second, and rings shells
  exactly once.

## [0.4.0] - 2026-10-16

First public release. Versions 0.1.0–0.3.1 were released before the
repository was public; their notes are not carried over.

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
  `plugin validate`, `debug paths`, `setup` (with `--redetect-icons`), and shell completions for all three
  shells.
- **Plugins.** Segment plugins loaded at runtime from a manifest; see
  [docs/dev/plugins.md](docs/dev/plugins.md).
- **Installation.** One-line installer (a release asset that installs exactly
  its release), release-archive installer, a Homebrew tap, a Fisher plugin,
  `gpy setup` to wire Fish, Zsh or Bash into its rc file, and uninstall
  scripts for Fish and POSIX shells.
- **Transient prompt** for Fish 4.2+ and Zsh.

### Security

- The socket is created with `0600` permissions; registration validates PIDs with `kill(pid, 0)`, so the agent
  only signals the caller's own processes.
- IPC messages are validated (path length, NUL bytes, control characters) and
  capped at 64 KiB. Repaint signals to a shell are throttled (150 ms default,
  `GPY_SIGURG_THROTTLE_MS`).
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

### Upgrading from 0.3.1

- `gpy setup --redetect-icons` re-runs Nerd Font detection and rewrites only `ui.show_icons`, in place. A first-time `gpy setup` still runs detection without overwriting an existing value.
- **Breaking:** `gpy-agent` keeps only `start`, `stop`, `status`, `oneshot`, `theme export` and `config reload`. Removed, with their replacements: `gpy-agent init` → `gpy setup` (`init --force` → `gpy setup --redetect-icons`); `gpy-agent theme validate`/`theme import` → `gpy theme validate`/`gpy theme import`; `gpy-agent palette …` → `gpy palette …`; `gpy-agent doctor` → `gpy doctor`; `gpy-agent config get`/`config list` → `gpy config get`/`gpy config show`.
- `gpy` output uses colored ✓/✗/! only for pass/fail/warn results; single-action confirmations are plain text. When stdout is not a terminal, `NO_COLOR` is set, `TERM=dumb`, or `ui.show_icons = false`, these print as `ok:`/`FAIL:`/`warn:`. `gpy segments` marks enabled segments with `[x]`.
- **Breaking:** the repaint throttle environment variable `GPY_SIGUSR1_THROTTLE_MS` is now `GPY_SIGURG_THROTTLE_MS`. The old name is no longer read.
- **Breaking:** the one-line installer is a release asset, not a file on `main`: use `curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh`, or `releases/download/vX.Y.Z/install-oneline.sh` for a specific release. The published copy has its release tag built in and installs exactly that release.
- `install-oneline.sh` installs the Fish, Zsh and Bash scripts from one archive, `gpy-shell-files.tar.gz`, after checking its published SHA-256, instead of downloading each script from a branch or tag unverified. `GPY_VERSION` can therefore only name 0.4.0 or later.
- Releases also publish `install-oneline.sh`, `uninstall.sh` and `uninstall.fish` (checksummed and attested like every other asset); the installer's uninstall hint points at them.

[Unreleased]: https://github.com/jpease/gpy/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/jpease/gpy/releases/tag/v0.4.0
