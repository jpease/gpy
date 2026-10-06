# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - YYYY-MM-DD

### Added

- Python language segment now reports the project virtualenv's interpreter
  version. Detection prefers a forwarded `$VIRTUAL_ENV` (fish/bash/zsh), then a
  project-local `.venv`/`venv` directory, reading the version from `pyvenv.cfg`
  (falling back to invoking the venv's `python`). Previously the agent daemon —
  which never inherits an activated venv — reported its own global interpreter
  regardless of the active venv. `Message::LanguageDetect` gained an optional
  `virtual_env` field (MINOR, back-compatible); a per-repo stash keeps the
  background refresh consistent with the synchronous reply.

- Config wizard master/detail layout (#400): the Theme/Palette/Segments
  option lists now render as stable vertical lists (one item per line, via
  `ratatui`'s `List`/`ListState`) instead of horizontally-wrapped text, and a
  new Detail panel describes whichever item currently has the cursor without
  requiring it to be selected first. Segment details show enabled state, the
  affected config field, what the segment contributes to the preview, and
  any known preview caveats (e.g. clock/duration/status use fixed sample
  values); Theme/Palette details show whether the highlighted item is
  currently selected and whether the selection has changed from the startup
  config. Below 80 columns the detail panel stacks below the option lists
  instead of squeezing beside them.

- Extend `is_first`/opening-cap suppression to the instant-prompt cache and
  `gpy-agent oneshot` fallback (#401), closing the gap noted below: the
  on-disk cache now writes `git`/`git_last`/`git_first`/`git_first_last` (and
  the `lang` equivalents) so a `git`/`language` segment that's first renders
  correctly from a cold cache instead of self-correcting after the first live
  repaint. `gpy-agent oneshot` subcommands gained a `--first` flag, threaded
  through the Fish/Bash/Zsh oneshot-fallback path. Also fixes a pre-existing
  gap where Bash/Zsh's `git`/`language` segments never threaded `is_last`
  into the cache-suffix lookup at all (Fish already did).

- Bound the daemon's language-detection scan and its shutdown blast radius
  (#391, follow-up to #390): `Detector::detect_directory_bounded` now falls
  back from `Content` to `Markers` mode when a real repo's non-ignored file
  count exceeds a fixed budget (20,000 files), not just when the path is
  outside a git repo — hyperpolyglot's scan has no cancellation hook or size
  cap of its own, so this bound is checked before handing off rather than
  mid-walk. Separately, `gpy stop`/agent shutdown now bounds Tokio runtime
  teardown with `shutdown_timeout` instead of relying on `Runtime::drop`,
  which previously blocked process exit until every in-flight
  `spawn_blocking` job finished (including an unbounded scan).

- Config wizard clock preview + position-based opening-cap suppression: the
  wizard's live preview now shows a fixed demo time for the `clock` segment
  (previously invisible in the preview since clock has no agent-side
  template — the preview title now reads "clock shows a fixed demo time"
  instead of "not shown here"). Separately, whichever segment ends up first
  in the enabled chain — by position, not by hardcoding "clock" — now
  suppresses its opening powerline cap, matching how the closing cap already
  works for the last segment. New `is_first`/`$sep_open` mirror the existing
  `is_last`/`$sep_close` mechanism end-to-end: IPC protocol (`Message`
  variants), the template engine, `config/themes/default.toml`, and the
  Fish/Bash/Zsh shell integrations. See `SCHEMA_EVOLUTION.md` and #401 for the
  known instant-prompt-cache gap this doesn't yet cover.

- Best-in-class git status segment (#240): the default theme's `[segments.git]`
  `format` now composes `$state` and `$stash` blocks alongside the existing
  `$branch`/`$ahead_behind`/`$status`, each collapsing to nothing when empty so
  a clean, unstashed repo renders byte-identical to before. `$state` shows an
  in-progress merge/rebase/cherry-pick/etc. (with step/total progress for an
  interactive rebase, e.g. `↻ REBASING 3/5`); `$stash` shows a stash count
  (e.g. `≡2`), gated by the new `[git] stash_enabled` config flag (default
  `true`) so the extra `git stash list` subprocess call can be disabled
  entirely. Detached HEAD is now tracked via an explicit `detached` flag
  instead of smuggling `"HEAD@<sha>"` into the branch string, and renders a
  distinct `$symbol` glyph. A new `[git] icon_set = "unicode" | "nerd_font"`
  option (default `unicode`) selects the glyph set for these three new
  indicators only (the four pre-existing status icons are unaffected).
  Color-state precedence grew from 4 to 6 buckets (`Conflicts > InProgress >
  Modified > UntrackedOnly > AheadBehind > Clean`), so untracked-only and
  in-progress repos are visually distinct from staged/unstaged changes out of
  the box (`untracked_only_*_color`, `in_progress_*_color` in
  `[segments.git]`). Themes can also set a per-state style attribute via a new
  `[segments.git.git_style]` subtable (e.g. `conflicts = "bold"`), mirroring
  the per-language `styles` mechanism (#231). All additions are additive and
  backward-compatible — existing themes render unchanged. (#240, #241–#248)

- Opt-in `hostname` segment (fish/bash/zsh), hidden by default and shown only over SSH
  (`show_always` opts into always-on display), matching Starship's `hostname` module
  defaults (`ssh_only = true`). Trims the hostname at the first `.` by default (FQDN →
  short name; `trim_at = ""` disables trimming). Renders pure-fish/bash/zsh by default;
  setting a theme `format` string switches to agent-rendered Starship-template parity.
  The Starship importer maps the `hostname` module to the new segment, inverting
  `ssh_only` onto `show_always` and preserving `trim_at`, `ssh_symbol` (as `icon`), and
  `format` (with `$ssh_symbol` renamed to `$symbol`). `config/themes/default.toml`
  ships a `[segments.hostname]` color block (`bg_color`/`text_color`) for users who opt in, but
  `hostname` is **not** added to any theme's default `enabled_segments` list. (#257, #258, #259)

- Per-language style-attribute overrides: themes can set `<lang>_style` (e.g.
  `java_style = "dimmed"`), exposed in language `format` strings as `$attr`. The
  starship preset now renders `java` as `red dimmed`, matching native Starship.
  The Starship importer preserves per-language attributes (previously dropped). (#231)

- Starship preset parity for git status and language modules. The `starship` theme now
  renders Starship-exact git status (`[!?]`-style symbols with **no** per-category
  counts), shows a language's symbol even when no version resolves, and excludes the
  fish-shell "language" (Starship has no fish module). Backed by new theme-owned,
  optional fields (each with a serde default, so existing themes are unchanged):
  `[segments.git]` `show_counts`, `staged_icon`, `unstaged_icon`, `untracked_icon`,
  `conflicts_icon`; `[segments.language]` `show_symbol_without_version`,
  `enabled_languages`. Added `tests/fish/starship_parity.test.fish`, which renders both
  prompts non-interactively and asserts parity against native Starship (skips when
  `starship` is not installed).

- `gpy theme import <starship.toml>` converts a Starship config into a GPY palette
  (`~/.config/gpy/palettes/<name>.toml`) and prompt theme (`~/.config/gpy/themes/<name>.toml`).
  Supports `--name`, `--force`, `--stdout`, and `--apply-layout`. Unsupported Starship
  modules/options produce grouped warnings on stderr; the import exits 0 on a lossy import.
  See `docs/dev/starship-import.md`. (#186)

- **base16/base24 palette color schemes** (SP4, #230): any of the 230+
  [tinted-theming](https://github.com/tinted-theming/home) schemes can now be
  imported with `gpy palette import <scheme.yaml>` and activated with
  `gpy palette use <name>`. Six scheme palettes ship as builtins:
  Catppuccin (latte / frappé / macchiato / mocha), Nord, and Gruvbox dark
  (medium). Two new standard color roles — `orange` (base09) and `brown`
  (base0F) — complete the base24 vocabulary. The `starship` preset migrates its
  raw 256-color indices (Swift 202, PHP 147, C++/C 149) to these named roles and
  recommends the new `starship` palette; `gpy theme use starship --force` activates
  both together for byte-identical vanilla parity. Swapping to any scheme palette
  with `gpy palette use <name>` recolors all 13 language indicators with no theme
  change. See `docs/dev/palettes.md` for the full role-vocabulary table and
  walkthrough.

- First-class color palettes: `gpy palette list/use/show/validate`, `ui.palette` config field, builtin `default` palette (#198).

- **Palettes now affect rendering** (SP2 #199): `config.ui.palette` is threaded through `PaletteManager` → `to_template_palette()` → template engine. `Color::Named` resolves through the active palette first (Starship semantics), falling back to standard ANSI names on a miss. The template+palette path is the color path for all agent-rendered segments.
  - `default.toml` and `text.toml` themes migrated to `format`+palette. Legacy `ColorSpec`→ANSI renderers (`render_repository_status`, `render_languages`, `color_to_ansi_fg/bg`) removed.
  - Golden tests pin canonical template output as a forward regression guard. No-format / template-error now yields an empty segment.
  - Template color validation resolves through the active palette; a color absent from both the palette and the standard ANSI name set is rejected.
  - The instant-prompt cache renders with the active palette; a `ui.palette` change invalidates and refreshes the cache (same path as a theme change).

- **Dead Client Cleanup**
  - Automatic pruning of dead Fish process PIDs from client registry
  - Periodic cleanup runs every 60 seconds in agent event loop
  - `ClientDirectory::is_client_alive()` method using `kill(pid, 0)` for liveness checks
  - `ClientDirectory::prune_dead_clients()` method for registry cleanup
  - Comprehensive test coverage for dead client detection and pruning

- **Core Functionality**
  - Fast Rust agent for Git status and language detection
  - Unix domain socket IPC for macOS and Linux
  - Real-time file system monitoring with debouncing (100ms default)
  - Memory-efficient caching with configurable TTL
  - Graceful shutdown handling (SIGTERM/SIGINT)
  - Debug logging infrastructure

- **Fish Shell Integration**
  - Modular segment system for prompt customization
  - Git status indicators (ahead/behind, staged, unstaged, untracked, conflicts)
  - Language detection with version info (Rust, Node.js, Python, Go, etc.)
  - Directory segment with path shortening
  - Live-updating clock segment with configurable second display
  - Duration segment showing command execution time
  - Status segment for exit code indication
  - Theme support with Nerd Font icons
  - Configurable segment ordering and separators

- **Live Updates**
  - File watcher sends SIGUSR1 to Fish processes on git changes
  - Clock timer sends periodic SIGUSR1 for time updates
  - Configurable clock update frequency (1s or 60s intervals)
  - Minute-aligned clock updates when seconds are hidden
  - Automatic prompt repainting via `commandline -f repaint`

- **Platform Support**
  - macOS support with Unix domain sockets
  - Linux support with Unix domain sockets
  - ARM64 architecture support (Apple Silicon, ARM Linux)
  - XDG Base Directory specification compliance
  - Agent auto-start via Fish event handlers

- **Output Formats**
  - JSON format for programmatic consumption
  - fish-ansi format with pre-rendered ANSI escape codes
  - fish-source format for direct Fish variable export (zero Python dependencies)

- **Development & Testing**
  - Comprehensive test suite (unit, integration, E2E)
  - E2E tests for signal delivery and agent auto-start
  - Manual testing protocol for visual validation
  - Performance regression detection
  - Clippy linting with strict warnings

- **Documentation & Distribution**
  - Installation script with binary download
  - Uninstall script for complete cleanup
  - README with configuration examples
  - Architecture documentation in module comments

### Changed

- The pinned Rust toolchain and the minimum supported Rust version are now
  1.99 (`rust-toolchain.toml`, `rust-version`, clippy `msrv`). Rust 1.99's
  `clippy::assert_is_empty` and `branches_sharing_code` findings are fixed.
- Language detection now runs on `gengo-language`'s matcher tables instead of
  `hyperpolyglot`, which is no longer a dependency (#523, epic #519). The
  reported languages and confidences are unchanged by design: detection keeps
  hyperpolyglot's resolution order (filename, then path glob, then extension,
  with a capped 51,200-byte read only when the path leaves the language
  ambiguous), so a `.py` file with a `#!/usr/bin/env ruby` shebang still
  reports Python. GPY now owns the directory walk itself — `ignore`'s parallel
  walker with GitHub Linguist's vendor and documentation exclusion globs
  vendored into `gpy-agent/src/language/filters.rs` — rather than borrowing
  hyperpolyglot's. The `gengo` wrapper crate added in #520 was dropped
  unwired: its `Gengo::analyze` reads every file in the tree (546 ms on 20k
  files against a 200 ms budget) and its default features reintroduce `gix`.

**Breaking shell-contract change (SP2, #199).**

> Pre-production breaking change (pre-authorized per epic). No compatibility shim.

For segments with a confirmed agent render path (**directory**, **duration**, **character**, **git**, **language**), the agent now returns pre-formatted ANSI directly. The following shell-side variables are **no longer exported** by `gpy-agent theme export`:

- `__color_directory_bg/fg`, `__color_duration_bg/fg`
- `__color_git_bg/fg` and all per-element git colour variables
- `__color_language_bg/fg` and all per-language colour override variables
- `__gpy_directory_format`, `__gpy_duration_format`, `__gpy_character_format` toggles

**Not changed**: `clock`, the exit-status block, and plugin/custom segments remain shell-rendered. Their `__color_*` exports (`__color_clock_bg/fg`, `__color_status_ok_bg/fg`, `__color_status_fail_bg/fg`, plugin `__color_<name>_bg/fg`) and the 8 delimiter color exports (`__segment_delimiter_color/bg`, `__prompt_open/close_color/bg`, `__prompt_base_bg/fg`) continue to be emitted unchanged.

The `GPY_SHOW_STATUS` indicator is now gated on the runtime agent-render outcome: when the prompt character is agent-rendered (exit-colored `❯`), the separate status indicator is suppressed to avoid duplication.

- Performance Improvements & Benchmarking:
  - Restored real-world Git status benchmarks (`git_status_cold`) using modern `gix` backend.
  - Implemented true IPC roundtrip benchmark (`ipc_roundtrip`) measuring Unix socket latency.
  - Updated performance budgets and added live enforcement in CI pipeline.
  - Performance confirmed at ~39µs for IPC and ~11.5ms for cold Git status (100 files).
  - Standardized benchmark commands under `just bench-<descriptor>` (e.g., `bench-rust`, `bench-shell`).

- Theme/config refactor:
  - Themes now group segment visuals under `[segments.<name>]`
  - All glyph fields renamed to `*_icon` and color fields standardized with `*_color`
  - New integration tests cover theme export, schema loading, and Fish prompt rendering
  - Config files expose `git.icons` and `language.icons` for semantic glyphs

### Removed

- `.gpy.toml` as a configuration source (#733). It was only ever resolved
  against the agent's launch directory and never hot-reloaded, so the prompt
  and `gpy config get` disagreed depending on the current directory. Move its
  settings to `~/.config/gpy/config.toml`, or point `GPY_CONFIG_PATH` at it.
- `install-dev.fish --bundle`, which produced payloads `install.sh` rejected
  (#810); use `just build-all-platforms`.
- Fish `prompt-config`, `prompt-theme` and `prompt-perf`, which always
  failed with exit 127 (#768).
- The `gpy_setup` and `gpy_config_validate` Fish functions, dead since
  `config.toml` replaced the `config.fish`-era layout they walked users
  through. Installers, the Homebrew formula and `fisher.json` no longer ship
  or reference them (#662). Use `gpy-agent init` / `gpy config wizard` in
  place of `gpy_setup`, and `gpy doctor` in place of `gpy_config_validate`.

- `gpy-agent oneshot git --changed-file` (#776). It reported one file's
  counts as the whole repository's status, and nothing used it.

### Fixed

- The directory segment rendered by the agent now shows the shell's logical
  `$PWD` instead of the symlink-resolved path (#697). `cd ~/app/current` (a
  symlink) showed the target's name, a `$HOME` reached through a symlink lost
  `~` contraction, and macOS `/tmp` showed as `/private/tmp`; IPC and oneshot
  output now match. Path validation, read-only checks and push-delivery
  matching still use the canonical path; the wire format is unchanged.
- Fish shells pick up a theme change made while the agent was stopped
  (#701). A new shell kept the stale theme it sourced, and open shells
  only re-registered after the agent restarted. The starting agent now
  rings `.reload` for tracked shells when its theme export actually
  changed (a plain restart writes nothing), and Fish re-applies a newer
  export after registering.

- The agent no longer signals an unrelated process that reused a dead
  shell's PID (#781). A shell tracking file older than its PID's current
  process is now removed with its flags instead of getting a `.reregister`
  flag and `SIGURG`. Each tracked shell is also nudged once per agent start
  instead of twice; the first periodic re-nudge comes 60 seconds later.

- An agent whose socket was removed or taken over by another agent now
  exits within 30 seconds (#779). A wedged agent that recovered after being
  evicted, or one whose socket was deleted, used to run forever where no
  command could reach or stop it. It never removes the newcomer's socket.

- `gpy start` and `gpy-agent start` return only once the agent answers on
  its socket (#741), and exit `1` with an `Error:` line when it dies or
  does not answer within 5 seconds. They used to exit `0` right after
  forking, before the socket existed and even when the agent never came up.

- The agent daemon no longer keeps its launch directory busy (#724). It
  now runs in `/`, so a volume you started it from can be unmounted while
  it runs. Relative `--socket`, `GPY_AGENT_SOCKET_PATH`, `GPY_CONFIG_PATH`
  and `GPY_DEBUG_LOG` values still resolve against the launch directory.

- Concurrent `gpy-agent start` runs no longer orphan a daemon (#723). A
  start that raced an eviction could unlink the replacement agent's socket
  and fork a third daemon, leaving one running on an unreachable socket.
  Starts on the same socket now serialize on a `<socket>.lock` file held
  until the new daemon has bound, and eviction only ever removes the socket
  (and version marker) it judged old. A start that finds the old agent
  already replaced by a responsive one exits `0` without forking.
- The Fish agent supervisor no longer writes to the terminal that started it
  (#767). Its stdin, stdout and stderr are now `/dev/null` and its working
  directory is `/`. Before, with `GPY_VERBOSE`/`GPY_DEBUG` set, its restart
  messages appeared in that terminal at random times.

- Fish no longer starts an agent supervisor when the agent is disabled
  (#699). With `[agent] enabled = false`, the first prompt spawned a
  supervisor that never exited, and a supervisor loop restarted the agent
  even when the shell had `GPY_AGENT_ENABLED=0`. The loop now re-reads both
  flags on every iteration, from the theme export the agent rewrites when
  `config.toml` changes, and exits within one check interval when either is
  `0`. The Agent-Free Mode docs now use the `config.toml` keys; the theme
  export overwrites a `set -gx` in `config.fish`.

- Fish registers with a running agent when the supervisor is disabled
  (#700). With `[agent.supervisor] enabled = false`, the per-prompt hook
  removed itself before registering, so a shell outside git and project
  directories never got live updates. Only the restart loop is skipped now.

- Zsh honours `[agent.supervisor] enabled = false`, and Bash and Zsh honour
  `check_interval_seconds` and `max_restart_attempts` (#762). Zsh decided
  whether to supervise before the theme export had loaded, so it kept
  restarting a stopped agent; its one startup start now runs after the
  export and is gated the same way. Both shells read the interval and cap
  from the exported `GPY_AGENT_SUPERVISOR_*` values on every check. With
  nothing configured, the fallback is now 30 s and 5 attempts (was 10 s and
  3). `GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS` and
  `GPY_SUPERVISOR_CHECK_MAX_ATTEMPTS` still override when set.

- Fish runs at most one agent supervisor per runtime root (#703). Shells
  that reached their first prompt together each started a supervisor, and
  only the last one was recorded in `supervisor.pid`. The supervisor now
  claims the file with fish's no-clobber `>?`, a loser exits, and a
  supervisor whose file names another process stops. A recycled PID in a
  stale file no longer suppresses supervision.

- `gpy-agent start` no longer evicts a newer running agent (#780). It
  replaced the daemon on any version difference, so two installs could flip
  it back and forth. Only an older or unreadable version is replaced now.
  An agent on a `--socket` / `GPY_AGENT_SOCKET_PATH` override keeps its
  version in `<socket>.version` instead of the default agent's marker.

- Zsh prints IPC requests, replies and cached output verbatim (#676).
  `echo` and `print` without `-r` interpreted backslash escapes, so a path
  containing `\` reached the agent as invalid JSON and replies containing
  `\c` or `\b` were truncated. Every site now uses `print -r --`.

- Bash no longer re-renders the prompt from a SIGURG trap (#678). The
  doorbell trap ran a full render re-entrantly, which hung bash 5 and looped
  bash 3.2 at an idle prompt, and it made `wait` return 144. Bash now has no
  URG trap; the agent's reload/reregister flag files are consumed at the next
  prompt, before it renders.

- Bash keeps `$_` across the DEBUG trap (#682), so `mkdir -p dir && cd $_`
  works on bash 4+.

- Bash measures the duration of the whole command line (#684). The start
  time is armed by a hook kept last in `PROMPT_COMMAND` and recorded once per
  command line, so other `PROMPT_COMMAND` entries, each simple command of a
  list or loop, and doorbells no longer reset it.

- Fish background git/language refreshes no longer block the prompt (#685).
  They are sent through a detached `socat`/`nc` pipeline, and with the agent
  down they fork nothing, so the directory segment still renders.

- Incremental git updates count conflicted files once and keep a staged
  deletion shadowed by an untracked file at the same path (#686). Counts are
  now derived from the per-file status map, as a full scan does.

- Unwatching one repository no longer removes another repository's
  overlapping OS watches (#687): nested repositories, submodules and linked
  worktrees keep their live updates. Watches are reference-counted per owner
  on every backend.

- Incremental git updates no longer diverge from a full `git status` (#711,
  #713, #775): files inside a new untracked directory are not counted twice,
  emptying that directory clears it, a filename starting with `:` is matched
  literally, a changed path with a non-ASCII name falls back to a full scan,
  and a TAB in a tracked filename no longer truncates its cache key. A
  differential test now checks the incremental pipeline against an
  independent parse of `git status` after random file operations.

- The Python segment falls back to `python3` when `python` is not on `PATH`
  (#688), as on stock macOS, Debian/Ubuntu and Homebrew.

- A failing or hung version probe is cached for 60 seconds instead of being
  re-spawned on every render or kept for 24 hours (#689). A hung tool no
  longer drops the IPC reply.

- `gpy theme import` fills modules absent from `starship.toml` from the
  builtin `starship` preset (#690), so git, directory and duration render
  instead of printing nothing.

- `gpy theme import` and `gpy palette import` refuse to shadow a builtin or
  plugin theme or palette unless `--force` is given (#691). The README's
  `gpy theme import ~/.config/starship.toml` example now passes `--name`.

- The wizard keeps git and language in `ui.enabled_segments` on save, and
  `gpy enable git|language` restores a segment missing from the list (#692).

- `install-dev.fish` deletes shadowing `gpy`/`gpy-agent` binaries only under
  `$HOME`, and deletes nothing when its install directory is not on `PATH`
  (#693).

- Linux release binaries are built against glibc 2.31 with
  `cargo zigbuild`, and a release gate rejects any binary that needs a newer
  glibc (#694). They previously required glibc 2.39 and failed on
  Ubuntu 20.04/22.04 and Debian 11/12.

- Failed prompt segments no longer print protocol JSON into the prompt
  (#680). An `ansi`, `bash-prompt` or `zsh-prompt` request that fails (a
  denylisted `cwd` such as `/etc`, a broken `.git`, git disabled, or no
  repository in the oneshot fallback) now gets an empty line, so the segment
  is omitted and Fish no longer memoizes the error as its directory segment.
  The request keeps its format when path validation fails; `json` clients
  still get `{"error": ...}`. A connection rejected as busy is closed without
  a reply instead of receiving a JSON error line.

- Zsh sends an integer `duration_ms` (#681). The command duration was
  computed in floating point from `$EPOCHREALTIME`, so the agent rejected
  every duration request and the oneshot fallback rejected the float too;
  zsh never showed a working duration segment.

- Bash and Zsh prompts mark agent color escapes as zero-width (#679): the
  `bash-prompt` and `zsh-prompt` formats wrap every SGR sequence in `\[ \]`
  and `%{ %}`, so readline and ZLE no longer count escape bytes as columns.
  Typed input no longer wraps early, and Ctrl-A, history recall and
  completion keep the cursor on the input line.

- Shell end-to-end suites pass from a checkout whose path contains a space
  (#822): tests pass paths to child shells as arguments instead of
  interpolating them into `-c` code, and a meta-check rejects any
  interpolating `-c` string under `tests/` and `scripts/`.
- Local Rust hooks, `just lint` and `quality-check.sh` fail fast when the
  active toolchain is not the pin in `gpy-agent/rust-toolchain.toml` (#823),
  naming the override (e.g. a stray `RUSTUP_TOOLCHAIN`), instead of reporting
  lint findings from the wrong compiler.
- `cd` within a repository no longer tears down and re-arms its file
  watches, and a shell that registers in another repository releases the one
  it was in (#718). A change written between the old unwatch and re-arm was
  never reported.
- Clients dropped by a repaint broadcast, or whose PID was reused by another
  process, are removed from the file watcher by the next pruning pass (#782),
  so their repository is no longer watched and re-scanned.
- Editing the active theme file refreshes `theme-export.{fish,bash,zsh}`
  and the instant-prompt caches before the reload doorbell (#710), instead of
  leaving the export one edit behind.
- A symlinked `config.toml` (stow/dotfiles layout) hot-reloads when it is
  edited through the link or at its target, or when the link is retargeted
  (#720).
- The agent picks up edits to the active palette file (and
  `gpy palette use <same name>`) without a restart, regenerating instant caches
  and repainting open shells (#772).
- `gpy theme use` and `gpy config set ui.theme` no longer report a false
  "Agent not reloaded" when `GPY_THEME_WATCH_POLL_MS` or
  `GPY_CONFIG_WATCH_POLL_MS` is set (#778); the poll thread stops promptly.
- The agent re-resolves its config file on every reload (#788): fixing a
  config that was invalid at startup applies it, deleting `config.toml` reverts
  to defaults, and a higher-priority config file that appears is picked up.
- Switching themes in an open Fish/Bash/Zsh shell no longer keeps the
  previous theme's clock format or Fish status icons (#791).
- An unchanged instant-prompt cache entry has its timestamp refreshed on
  every verified refresh (#704), so quiet repositories no longer fork a
  background refresh and a git recompute on every prompt.
- The instant-prompt cache no longer shows a stale git or language segment
  indefinitely after concurrent writes for the same repository (#706); the
  dedup record now always matches the file on disk.
- The agent recreates its instant-prompts cache directory when it is
  deleted while running (#707), instead of failing every cache write until
  restart.
- Instant-prompt cache writes no longer fail for repositories with long
  paths (~210–230 bytes), and one failing variant no longer stops the others
  from being written (#708).
- The instant prompt cache hits for paths containing `? * < > " |` on Linux
  and macOS (#705); the agent escaped those characters but the shells did
  not, so the cache always missed there.
- The instant prompt cache works for long project paths (#771): keys over
  200 characters are stored in 50-character chunk directories, so the
  language and git segments render there from the second prompt on. Shorter
  keys keep their existing file names.
- `gpy enable`, `gpy segments`, completion, `gpy doctor` and the wizard
  recognise the shipped `hostname` and `username` segments (#740). `gpy doctor`
  no longer fails on the starship preset.
- `gpy config set`, `theme use`, `palette use`, `enable`/`disable`,
  `lang versions`, `theme import` and the wizard edit `config.toml` in place
  (#730): comments, unknown keys, key order and unset defaults survive, writes
  are atomic, and a symlinked `config.toml` stays a symlink. `gpy config open`
  on a missing file creates a short header instead of a dump of every
  default.
- `gpy enable` and the config wizard keep the order of
  `ui.enabled_segments` and no longer drop entries the wizard does not offer
  (#739); the wizard preview follows the list order.
- `gpy theme use --force` replaces a palette written by the outgoing theme's
  recommendation, and lists a palette it kept under "Preserved your explicit
  settings" (#731).
- `gpy config get/set/show` cover every config key (#789): `ui.palette`, the
  git icon and ahead/behind keys, `language.detection_mode`, and the documented
  `gpy config set language.icons.<lang>`. `gpy config show` prints every field
  as valid TOML (`skip_paths = []`).
- The shipped `config/config.toml` sets `language.confidence_threshold` to
  its 0.1 default instead of 0.0 (#790).
- `gpy-agent oneshot --help` lists only formats the agent can render: the
  rejected `fish-ansi` and the unimplemented `zsh-source` are gone, and the new
  `bash-prompt` / `zsh-prompt` values appear (#756).
- The CLI reference no longer documents a nonexistent `gpy-agent restart`;
  use `gpy restart` (#783).
- The published IPC JSON schemas (`gpy-agent/schemas/`) match the agent:
  every op and `format` value, and the reply shapes the `json` format really
  sends (#760). The wire protocol is unchanged.
- `gpy stop` and `gpy-agent stop` exit 1 with an `Error:` line when the agent
  does not answer the shutdown request or keeps running (#742). A stale socket
  file is removed and reported as "Agent is not running (stale socket
  removed)" instead of `kill` advice, and `gpy restart` continues to start
  after a failed stop.
- Bold, underline and background no longer leak from one styled template
  group into the next (#752). The encoder resets only when the previous style
  would leak, so the shipped themes' bytes are unchanged.
- Bash chains a pre-existing DEBUG trap (e.g. bash-preexec) and EXIT traps
  containing quotes (#683). gpy's DEBUG trap is now installed at the first
  prompt, where bash exposes the prior trap; re-sourcing `gpy.bash` keeps the
  chain.
- Bash `set -u` and zsh `setopt nounset` shells no longer print
  unbound-variable errors on every prompt when `XDG_RUNTIME_DIR` or
  `XDG_CACHE_HOME` is unset, and zsh registers its exit hook (#698).
- Bash `PROMPT_COMMAND` hooks that run after gpy see the command's real `$?`
  instead of 0 (#761).
- Zsh no longer leaks a `gpy_cache_*` temp directory on every re-source,
  `exec zsh` or killed shell (#763); directories left by dead shells are
  removed at the next start.
- GPY's fish `conf.d` entry point no longer loads in non-interactive shells
  (`fish -c`, scripts, the supervisor child) (#769). `fish -i -c fish_prompt`
  still previews the prompt.
- The fish prompt no longer forks `rm` on every render in disabled-footprint
  mode (#770).
- Git prompts no longer show another repository's status when `GIT_DIR`,
  `GIT_WORK_TREE`, `GIT_INDEX_FILE` or a similar repository-local variable is
  inherited by the shell or agent (#714).
- gpy no longer overrides a `core.untrackedCache` set at any scope (#715);
  it enables the untracked cache only where the key is unset. Set it to
  `false` to opt out.
- The bash/zsh basic and parity test suites no longer leak a `gpy-agent`
  daemon on every gate run (#750); the gate also runs them against the
  checkout's debug binary instead of whichever `gpy-agent` is on `PATH`.
- `scripts/quality-check.sh --fix` changes to the repository root before
  formatting (#811); run from another directory it reformatted that
  directory's fish files.
- A missing `fish_indent` or `shellcheck` is reported as a `SKIP:` line by
  `scripts/quality-check.sh` and fails the gate under CI (#813), instead of
  passing silently.
- The installers quote the `source` path they write into rc files (#746), so
  a config directory containing a space no longer breaks every shell start.
  A directory containing `"`, `$`, a backtick or a backslash is refused before
  anything is written.
- The oneline Bash install no longer creates `~/.bash_profile` (which shadowed
  `~/.profile`) when `~/.bashrc` is absent (#747). gpy loads from `~/.bashrc`
  in login and non-login shells, and uninstall removes startup files the
  installer created.
- The oneline Zsh install and both uninstallers honour `ZDOTDIR`, using
  `${ZDOTDIR:-$HOME}/.zshrc` instead of a `~/.zshrc` zsh never reads (#748).
- Template style strings are case-insensitive (`Bold Red`, `BOLD red`), and
  `none` / `fg:none` empty the whole style while `bg:none` clears only the
  background, as in Starship (#794). Palette references keep their case.
- A `.` after a bare template variable is literal text (#796): `$branch.`
  renders the branch followed by a dot instead of nothing.
- `gpy theme import` applies Starship's default styles for `directory`,
  `cmd_duration` and `hostname` when the module sets no `style` (#734), and a
  partially configured `[git_branch]`, `[git_status]`, `[directory]` or
  `[cmd_duration]` keeps the preset's other fields (git status icons, colours)
  instead of replacing them.
- `gpy theme import` handles Starship character symbols with surrounding
  whitespace (`[➜](bold green) `) instead of storing the markup as literal
  text, and warns on symbols it cannot represent (#735).
- `gpy theme import` carries Starship's `add_newline = false` and
  `$line_break` (or the default two-line layout) into `ui.add_newline` and
  `ui.two_line`, and no longer warns that `line_break` is unsupported (#738).
- `gpy theme import` accepts `fg:`/`bg:`-prefixed and palette-alias colours
  in language and character styles instead of rejecting them as invalid
  (#792).
- Zsh and Bash wait up to `GPY_IPC_TIMEOUT_MS` for agent replies instead of
  a hard 100 ms, and zsh no longer forks a blocking `gpy-agent oneshot` when a
  connected agent replies late (#757).
- Bash and Zsh no longer treat an agent error reply to `register` as a
  successful registration (#758), so the next prompt retries.
- A rejected workspace sync (for example `cd /etc`) no longer drops the
  shell's agent registration in Fish, Bash or Zsh (#764); only a
  "not registered" reply triggers re-registration.
- Fish shell-rendered segments (status, clock, hostname, username) that
  follow an agent-rendered segment keep their opening cap (#765).
- The bash/zsh basic and parity test suites no longer leak a `gpy-agent`
  daemon on every gate run (#750); the gate also runs them against the
  checkout's debug binary instead of whichever `gpy-agent` is on `PATH`.
- `scripts/quality-check.sh --fix` changes to the repository root before
  formatting (#811); run from another directory it reformatted that
  directory's fish files.
- A missing `fish_indent` or `shellcheck` is reported as a `SKIP:` line by
  `scripts/quality-check.sh` and fails the gate under CI (#813), instead of
  passing silently.
- Windows-shaped `XDG_*` values (`C:\…`, `C:/…`, `\\server\share`) are
  ignored on Linux, macOS and WSL (#774), so the agent uses the same socket
  and caches as the shells instead of directories relative to its working
  directory.
- The Fish, Bash and Zsh prompts no longer end with a dangling separator
  when the git or language segment passes detection but has nothing to show
  yet (#766); the neighbouring segment closes the line instead.
- The shell language pre-filters use the agent's own marker list, exported
  with the theme (#785), so non-git projects marked only by `setup.py`,
  `Package.swift`, `Rakefile` and similar files show their language.
- `gpy theme import` no longer inserts an unstyled space between the
  `git_branch` and `git_status` formats (#795), so powerline and Pure-style
  presets import without a gap.
- Fish `prompt-debug validate` no longer reports false errors, and
  `prompt-debug cache`/`vars` print live values (#768).
- `gpy plugin validate` and plugin discovery reject a `plugin.toml` whose
  `provided_segments` is missing, misspelled or empty, instead of loading it
  as ready with no segments (#787).
- The Fish and shell E2E suites rebuild the debug `gpy`/`gpy-agent` before
  running, honouring `CARGO_TARGET_DIR`, instead of reusing a stale binary
  (#812).
- Ahead/behind counts clamped by `git.max_ahead_behind` render with the
  documented trailing `+` (e.g. `↓100+`) (#717).
- `gpy doctor` no longer fails when the agent is disabled with
  `agent.enabled = false`; it reports the agent as disabled via config (#799).
- Fish dynamic completions (theme, palette and segment names) load in real
  sessions (#702): the installers append them to the autoloaded
  `completions/gpy.fish` instead of an unloadable `gpy-dynamic.fish`.
- uv and virtualenv venvs (`pyvenv.cfg` `version_info`) are read without
  spawning Python, and the interpreter fallback for other venvs is memoized
  instead of running on every render (#728).
- The git segment shows `cherry-picking`/`reverting` when a multi-commit
  sequence is in progress after a manual commit, and in reftable repositories
  (#716).
- `gpy doctor` no longer reports a missing or unparsable theme as
  "Invalid template" with template-fix advice; the template check shows as
  skipped because the theme failed to load (#803).
- The `install_from_source_docs` test runs sealed from the caller's
  environment and stops only its own sandbox agent (#749).
- `magenta`, `bright_*`/`br*` and `gray`/`grey` render as ANSI colours in
  templates (#732), so the default theme's rebase/merge pill keeps its
  background. Palettes with unresolvable or cyclic references are rejected
  when loaded instead of rendering nothing.
- `install-dev.fish` no longer leaves the agent running with `GPY_DEBUG_LOG`
  pointing at an ever-growing `/tmp/gpy-verify-<pid>.log` (#743).
- The Python segment shows the active conda/mamba environment's version in
  Bash, Zsh and Fish (#729): `CONDA_PREFIX` is forwarded for non-`base`
  environments when `VIRTUAL_ENV` is unset.
- Edits inside a submodule or nested repository update the parent
  repository's prompt status, and reverting them clears it (#712).
- `gpy theme import` prints `gpy palette use` before `gpy theme use`, and
  the docs match (#793), so the printed activation steps work for imports
  that reference palette colours.
- The config wizard preview always shows the `❯` character line and draws
  `status` as a ✔/✖ pill in the segment chain (#800).
- The installers no longer destroy a symlinked `fish_prompt.fish`, and
  uninstall restores it and backups made by older `install-dev.fish` runs
  (#744).
- The `gpy-agent oneshot lang` fallback detects languages at the git root
  (#784), matching the agent when the shell is in a repository
  subdirectory.
- `gpy palette import` names the palette from the scheme's `slug` when
  present and keeps non-ASCII letters otherwise (`Rosé Pine` → `rose-pine`
  with a slug, `rosé-pine` without) (#797).
- `gpy config wizard` shows its save confirmation and any "Agent not
  reloaded" notice after exiting instead of losing them with the alternate
  screen (#801).
- The installer environment overrides (`GPY_SHELL`, `GPY_VERSION`,
  `GPY_NERD_FONT`) are documented on the `sh` side of the pipe
  (`curl … | GPY_VERSION=v1.2.3 sh`), where the installer sees them (#745).
- `docs/dev/palettes.md` and `docs/dev/starship-import.md` describe the
  palette and Starship-import behaviour the code actually has (#798).
- A `.ruby-version` written as `ruby-3.2.2` or `ruby-3.2.2@gemset` shows
  `ruby 3.2.2` instead of `ruby ruby-3.2.2` (#786).
- Selecting a user theme that fails to parse in `gpy config wizard` no
  longer aborts the session; the selection reverts and the error is shown
  (#802).
- `gpy theme import --help` and `gpy palette import --help` say that
  `--force` also lets an import shadow a builtin or plugin theme or palette
  (#821).
- Incremental git updates see edits and reverts of files whose on-disk
  case differs from git's on case-insensitive repositories (#713).
- `gpy debug prompt` exits 1 with an `Error:` line when no agent is running,
  and its help and docs describe the round trips it actually times (#804).
- Editing `.node-version`, `.nvmrc`, `mise.toml` or `.mise.toml` refreshes
  the displayed language version immediately (#725).
- `install-oneline.sh` checks a downloaded agent or `gpy` binary before
  replacing the installed one (#806), so a binary that cannot run on the host
  no longer overwrites a working install.
- `gpy plugin validate plugin.toml` works from inside the plugin directory
  instead of failing with "Plugin root '' is not a directory" (#805).
- The directory segment no longer renders `/` for a working directory with
  a trailing slash or `/.` (#755).
- `install.sh` and `install-oneline.sh` keep only the most recent
  `gpy-agent` and `gpy` backup in `~/.local/bin` instead of adding a full
  copy on every run (#807).
- The language segment in non-git directories picks up added or removed
  project files within about 30 seconds instead of keeping the first
  detection until the agent restarts (#709).
- A deactivated Python virtualenv no longer reappears in the prompt after
  background refreshes (#726).
- The oneline Fish install aborts, leaving `config.fish` untouched, when
  `gpy_init.fish` or `fish_prompt.fish` fails to download (#808), instead of
  installing a config that sources a missing file.
- `ui.directory.truncate_to_repo` no longer anchors at a git repository
  rooted at `$HOME` (dotfiles), so paths under home show as `~/…` (#753).
- Repositories that track gpy themes, or are rooted at `gpy/themes`,
  live-refresh the git segment, including on `.git/HEAD` changes (#777).
- `ui.directory.display = "abbreviated"` keeps a hidden directory's dot plus
  one character (`~/.c/fish`) and no longer splits multi-codepoint leading
  characters such as flag emoji (#754).
- The manual and from-source Zsh/Bash install steps write a
  `# >>> gpy-init >>>` marker block, so `scripts/uninstall.sh` removes the
  source line instead of leaving it behind (#809).
- `prev_fg`/`prev_bg` resolve through the active palette (#737), so
  powerline chevrons match the neighbouring segment's colour.
- The `uninstall.fish` confirmation listing shows the `conf.d/gpy_init.fish`
  path instead of an empty item (#814).
- Multi-language segments chain each language pill with its own caps and
  gaps instead of giving every pill the whole segment's first/last position
  (#751).
- Edits to tracked files under `vendor/`, `dist/`, `build/` or `target/`
  refresh the git segment (#719); only gitignored paths are filtered.
- `gpy theme validate` and `gpy theme use` reject broken `<lang>_style` and
  `git_style` values and name the field (#736), instead of activating a
  theme whose segment renders empty.
- Non-git projects show their language in subdirectories without source
  files (#727): detection runs at the nearest ancestor with a project marker
  file, never above `$HOME`.
- `just install` runs `install-dev.fish` from a checkout, and
  `just build-all-platforms` stages the agent and `gpy` CLI with `.sha256`
  sidecars that `install.sh` accepts (#810).
- Socket requests with format `fish`, `bash-source` or `zsh-source` get a
  JSON `{"error": ...}` reply instead of no reply (#759).
- The poll watcher backend sees edits inside directories created after a
  repository was armed (#721), including new branch ref directories.
- `git.skip_paths` honours `~/` and symlinked entries and is applied by the
  registration scan, watcher refreshes and the oneshot fallback too (#696).
- `ui.show_icons = false` no longer emits Nerd Font powerline caps or
  private-use segment delimiters (#695), so prompts render without a Nerd
  Font.
- The Starship importer detects style attributes case-insensitively, so
  `Bold green` imports as bold (#828).
- Bash and Zsh no longer re-send a rejected workspace sync (for example
  `cd /etc`) on every prompt; it is sent once per directory visit (#833).
- `gpy theme import` maps Starship's `$all` to the default segment order
  instead of warning that it is an unsupported module (#827).
- The shell-test runner pins `XDG_RUNTIME_DIR` inside its hermetic root, so
  test agents are stopped on systems that set it (#824).
- `gpy theme import` sets Starship's default hostname `ssh_symbol` for
  SSH-only hostname modules, so imported themes show the globe over SSH
  (#826).
- The shell e2e harness sends `TERM` and waits before falling back to
  `KILL` when it stops a test agent (#825).
- `scripts/build-release-binaries.sh` builds Linux binaries against the
  release glibc floor with cargo-zigbuild and checks them, matching the
  release workflow (#818).
- A relative `GPY_CONFIG_PATH` is resolved against the current directory
  when read, so `gpy-agent status` reports and watches an absolute path
  (#733).
- The poll watcher fallback detects same-second rewrites of ref and config
  files, and no longer wakes a repository for a directory's own mtime change
  (#817).
- A failed recursive inotify worktree watch no longer leaks its partial
  watches after the repository's last client unregisters (Linux) (#722).
- `exec fish` / `exec bash` / `exec zsh` no longer closes the terminal
  (jpease/gpy-archive#674). The re-exec'd shell keeps its PID and stayed
  registered, so the agent's SIGUSR1/SIGUSR2/SIGALRM (all terminate by
  default) could hit it before its handlers were installed. Agent→shell
  notifications now use a single SIGURG (ignored by default) plus empty
  `<pid>.reload` / `<pid>.reregister` flag files in the runtime `shells/`
  directory; a plain SIGURG means repaint. The IPC protocol version is now 2:
  a running agent from an older version is restarted by the shell's protocol
  check. The `agent.restart.marker` file is removed. See
  [ADR-0007](docs/dev/adr/adr-0007-sigurg-doorbell-notifications.md).

- Fish uninstaller prompt preservation (#667): `scripts/uninstall.fish` now
  checks whether the destination `fish_prompt.fish` already exists (as an
  unrelated custom prompt, symlink, directory, or other object) before restoring
  a backup, preserving the existing file and leaving all backups untouched.
  Backup restoration now occurs only into an absent destination.

- Fish uninstaller backup timestamp selection (#670): `scripts/uninstall.fish`
  now chooses the latest backup by comparing the timestamp suffix
  (`YYYYMMDD_HHMMSS`) descending across both installer conventions
  (`fish_prompt.fish.backup.<stamp>` and `fish_prompt.fish.gpy-backup.<stamp>`),
  with deterministic `.gpy-backup.` preference on equal stamps, preventing older
  `install.sh` backups from incorrectly overwriting newer `install-oneline.sh`
  backups.

- Multi-shell global uninstallation and complete startup cleanup (#671): both
  `scripts/uninstall.sh` and `scripts/uninstall.fish` now perform global
  uninstallation for the current user's install across all supported shells
  (Fish, Bash, Zsh), removing integration files, stopping agent and Fish
  supervisor processes, and cleaning delimited `# >>> gpy-init >>>` blocks from
  all existing startup files (`~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, and
  `config.fish`).

- Theme validation segment coverage (#669): `validate_segment_templates` now
  validates `clock`, `hostname`, and `username` format templates alongside
  `git`, `language`, `directory`, `duration`, and `character`, preventing themes
  with invalid colors or broken templates from passing validation or being set
  via `gpy config set ui.theme`.

- Prospective activation validation in `theme use` (#668): `gpy theme use` and
  `gpy theme use --force` now validate the candidate theme's segment format
  templates against the prospective active palette before modifying `config.toml`,
  preventing activation of themes with broken templates or unresolved color roles
  and preserving configuration on validation failure.

- Empty and invalid theme name rejection in `theme new` (#672): `gpy theme new`
  now validates destination theme names against the `ThemeName` invariant
  before creating directories or files, rejecting empty names, whitespace-only
  names, and names containing invalid characters without creating unusable
  `.toml` files.

- Release workflow rebuilt for the current repository layout (#492). It
  validated root-level `core/`/`segments/`/`themes/` Fish paths that moved
  under `fish/` long ago, `apt-get install`ed a nonexistent `timeout` package,
  read `Cargo.toml`'s version without ever comparing it to the tag, copied
  every packaged file with `2>/dev/null || echo "... may be missing"` so an
  archive containing no shell files still went green, dropped all five agent
  binaries onto the same `bin/gpy-agent` path that `install.sh` never looks
  for, and published notes linking three deleted files
  (`CROSS_PLATFORM_TESTING.md`, `config.example.fish`, `REFACTOR.md`) plus a `.deb`, a PKGBUILD, a Homebrew tap, and a crates.io
  package this project does not ship. Packaging and release-note generation
  now live in
  `scripts/package-release.sh` and `scripts/release-notes.sh` — fail-closed,
  with archive contents asserted after the fact — and are exercised on every
  quality-check run by `tests/bash/release_packaging.test.bash` instead of
  only when a tag is pushed. Release notes are derived from the CHANGELOG
  section for the version and from the binaries actually built; the tag must
  agree with `gpy-agent/Cargo.toml`, `fish/fisher.json`, and `CHANGELOG.md`
  before anything is built.

- `fish/fisher.json` listed a nonexistent top-level `init.fish` and a build
  artifact path (`gpy-agent/target/release/gpy-agent`) in its `files` array,
  neither of which matches the installed plugin layout. It now lists the real
  tree: `conf.d/`, `core/`, `segments/`, `functions/`, `completions/`.

- Config wizard preview: the `clock` segment never rendered its opening/closing
  powerline cap glyphs, even for themes (including the built-in `default`
  theme) whose `ui.prompt_open`/`ui.prompt_close`/`ui.segment_open`/
  `ui.segment_close` icons are real glyphs, not empty — a visible mismatch
  from the live prompt's Fish-rendered clock (`gpy_section_standalone` in
  `fish/core/renderer.fish`), which always draws these caps. The wizard's
  clock preview now resolves and renders the same caps, including the
  `match_bg`/`match_text`/`transparent` color-keyword resolution and the
  same-background gap space before a non-last closing cap.

- Truncated IPC responses could be rendered into the prompt as raw partial ANSI (#300): the Fish/Bash/Zsh IPC clients treated any non-empty socket read as a complete response, so an agent that wrote a partial frame (e.g. `{"status":"o`) and then stalled or died mid-write handed that exact truncated string to callers — for `character`/`duration` (raw-ANSI) ops, straight into `PS1`/`PROMPT`, corrupting terminal rendering. All three clients now validate that the newline frame terminator was actually received (via `read`'s EOF-before-delimiter exit status) before accepting a response, and discard/fall back otherwise. No wire-protocol change — the agent already terminated every response with `\n`.
- Fish's per-prompt agent autostart had no backoff (#301): `__gpy_start_supervisor_on_prompt` only removed itself once `__gpy_register_with_agent` succeeded, so a crash-looping agent binary (starts, then immediately exits) got re-forked on every single prompt and stalled each one for `GPY_AGENT_START_DELAY_MS` (200ms) indefinitely. The hook now tracks an attempt counter and a rate-limit timestamp (`GPY_AGENT_AUTOSTART_MAX_ATTEMPTS`, default 3; `GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS`, default 10s) and disables itself for the shell session once the attempt limit is exceeded, leaving the background supervisor loop's own backoff to keep retrying.

- Memory leak from accumulated dead PIDs when Fish processes crash or are killed without unregistering
- Wasted signal delivery attempts (SIGUSR1) to non-existent processes

### Security

- Bash and Zsh prompts no longer expand directory, branch or theme text as
  prompt code (#677). A directory or branch named `$(cmd)`, `` `cmd` ``,
  `$VAR`, or containing `\u`/`%_` prompt escapes ran or expanded on every
  prompt draw. The agent gained `bash-prompt` and `zsh-prompt` output formats
  that render the same colors as `ansi` but escape all segment text for the
  consuming shell; only the clock's `\D{…}`/`%D{…}` live-time token is left
  for the shell to expand. Bash and Zsh request these formats (including the
  oneshot fallback), the instant-prompt cache writes one file per format
  (`.ansi`, `.bash`, `.zsh`) and each shell reads only its own, and the
  integrations pin `promptvars` (Bash) and `prompt_subst`, `prompt_percent`,
  `no_prompt_bang` (Zsh). Fish output (`ansi`) is unchanged.

- Release downloads are verified, and installation fails closed when they
  cannot be (#494). Every release now publishes a SHA-256 sidecar beside each
  binary and archive, an aggregate `SHA256SUMS` manifest, a CycloneDX SBOM
  (`gpy-sbom.cdx.json`), and a GitHub build-provenance attestation for each
  artifact.
  - `install-oneline.sh` and `install.sh` verify every binary before
    installing it. A missing checksum is refused exactly like a mismatched
    one: a release that publishes no verification metadata is
    indistinguishable from one whose metadata was stripped in transit. A
    machine with neither `sha256sum` nor `shasum` aborts before downloading
    anything, and a failed verification leaves any existing installation
    untouched.
  - `install-oneline.sh` no longer falls back to
    `raw.githubusercontent.com/jpease/gpy/main/bin/<asset>` or to
    `VERSION="main"`. Both pointed installs at a mutable branch with no
    verification; the binary path had never existed, so the fallback only
    turned a clear failure into a confusing one. A release install now
    resolves a tag or stops.
  - Quarantine handling narrowed from `xattr -c`, which stripped every
    extended attribute, to removing `com.apple.quarantine` — and only after
    the binary's checksum has been verified.
  - An absent `gpy` CLI asset is still non-fatal (the prompt does not need
    it); a `gpy` asset that downloads but fails verification is not.
  - Code signing and notarization remain deliberately undone, and are now
    documented as such in `SECURITY.md` along with what that means on macOS
    and Windows.
  - `scripts/quality-check.sh` gained a shellcheck gate over the installation
    and release scripts.

- Unix socket permissions (0600, user-only access)
- PID validation via OS-enforced kill() permissions
- Input validation for all IPC messages (path length, null bytes, control characters)
- 64KB message size limit to prevent memory exhaustion
- Rate limiting for client signals (50ms minimum interval per client)
- No external runtime dependencies

---

## Development Notes

This initial release establishes GPY as a high-performance Unix-based Fish shell prompt enhancement. Key highlights:

- **Performance**: Sub-millisecond IPC latency via Unix domain sockets
- **Live Updates**: SIGUSR1-based prompt updates for git changes and clock ticks
- **Zero Dependencies**: Removed Python dependency via fish-source format
- **Architecture**: Well-documented threading and debouncing rationale in module comments
- **Maintainability**: Comprehensive test coverage with E2E validation

---

[0.1.0]: https://github.com/jpease/gpy/releases/tag/v0.1.0
