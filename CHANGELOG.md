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

- The `gpy_setup` and `gpy_config_validate` Fish functions, dead since
  `config.toml` replaced the `config.fish`-era layout they walked users
  through. Installers, the Homebrew formula and `fisher.json` no longer ship
  or reference them (#662). Use `gpy-agent init` / `gpy config wizard` in
  place of `gpy_setup`, and `gpy doctor` in place of `gpy_config_validate`.

- `gpy-agent oneshot git --changed-file` (#776). It reported one file's
  counts as the whole repository's status, and nothing used it.

### Fixed

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
