# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.1] - 2026-10-27

v0.7.0 was tagged but never published: its release workflow did not install
`cargo-machete`, which the gate requires under CI. v0.7.1 is the same code.

### Added

- Script plugin segments (`entry_type = "script"`) now render in Bash, Zsh
  and Fish (#997). One Bash runner (`bash/core/g_runner.bash`, copied
  byte-identically to Zsh and Fish) runs them: inside the shell in Bash, in
  a Bash child in Zsh and Fish (the first Bash 4 or later on `PATH`, else
  `/bin/bash`; without any Bash, script segments are disabled). Each script
  starts as `sh -c PROLOGUE SCRIPT` from a clean environment, with declared
  values read on descriptor 3 (never argv; a folder-scoped segment's `$PWD`
  is never in argv either). Output is capped at 4096 bytes, C0/DEL stripped
  locale-independently, hidden on a C1 control and escaped once for the
  shell before it is cached in shell variables (no cache files). Scripts run
  at most once per command (repaints reuse the text), after `require_env`,
  the trigger scan (30 ms bound, names matched literally) and the cache;
  `require_command` shares the script's process group and deadline. Each
  segment has its `timeout_ms` from one shared start (at most 500 ms, at
  most 40 segments per prompt), with TERM/CONT/KILL of its process group and
  of leftovers after a normal exit. Failed scripts are cached as hidden;
  timeouts and Ctrl-C are not. Two consecutive scan timeouts, or two
  consecutive run timeouts or over-cap outputs, disable a segment for the
  shell. With Bash 3.2 (stock macOS) Ctrl-C during a slow script waits for
  its timeout. See SECURITY.md "Script Segment Trust Model".
- Theme export writes a script-segment block (`__gpy_script_sh`,
  `__gpy_script_segments` and 15 `__gpy_script_<tok>_*` variables per
  segment) for Bash, Zsh and Fish, the contract the shell runners consume.
  Export re-resolves `command` and `require_command` each time and skips a
  segment whose script, or any directory up to the plugin root, is not owned
  by you or root or is group- or world-writable; the shells run the exported
  canonical path without resolving it again.

### Changed (BREAKING)

- The `username` and `hostname` segments are now bundled G (script) plugins
  (`plugins/username`, `plugins/hostname`, #1003, part of #994). The built-in
  segments, resolvers, `__gpy_request_hostname`/`__gpy_request_username`, the
  per-shell `hostname`/`username` segment files and
  `gpy-agent oneshot hostname|username` are removed. The plugins are found in
  `$GPY_BUNDLED_PLUGIN_DIR`, default `$XDG_CONFIG_HOME/gpy/bundled-plugins`,
  which `gpy setup` (and so every installer) fills from `<source>/plugins`;
  release archives, `gpy-shell-files.tar.gz` and the Homebrew formula ship
  that tree, and `scripts/uninstall.sh` removes it. **Upgrading:** hostname
  `show_always` and `trim_at` and username `show_always` move from the theme
  to `config.toml` `[plugin.hostname]` / `[plugin.username]`; the theme keys
  `format` and `user_format` for these segments are gone (pills are drawn by
  the plugin framework, so the Starship theme's hostname and username are
  flat pills); a variant icon is set with the new `<variant>_icon` key (for
  example `ssh_icon`). Run `gpy setup` after upgrading.
- On Bash 5 the grace KILL after a script segment's timeout is now
  asynchronous, as it already was on Bash 3.2: the leftover KILL can run up
  to 50 ms after the prompt is drawn instead of delaying it.
- A script segment that is the last segment now draws its closing cap on the
  terminal default background (as the old username/hostname pills did), not
  on the `ui.prompt_close` background; built-in segments are unchanged. A
  theme reload unsets the previous script segments' style variables
  (`__gpy_segment_<tok>_*`, `__color_<tok>_*`, `__icon_<tok>*`) before the
  export is sourced, so a dropped style field no longer lingers.
- `gpy plugin new` now scaffolds a script plugin (`plugin.toml`,
  `segments/<segment>.toml`, `bin/<segment>.sh`) instead of a Fish file.
  `gpy plugin validate` lints each script with `shellcheck -s sh` or
  `dash -n` (a failure fails validation) and warns about undeclared `$VAR`
  uses; `gpy plugin list` shows script segments; `gpy doctor` runs each
  enabled script segment once with the runner's clean environment, working
  directory and `timeout_ms` kill and reports the time. Example plugins
  (Kubernetes context, username, hostname) are in
  `docs/dev/examples/plugins/`.
- `GPY_AGENT_ENABLED` and `GPY_AGENT_SUPERVISOR_ENABLED` now follow one rule
  in Bash, Zsh and Fish: a flag is off only when set to `0` or `false`.
  `false` now disables the agent in Fish (it stayed on before), and values
  other than `0`/`1` (for example `2`) now leave the flag on in Bash and Zsh.
  An unset `GPY_AGENT_SUPERVISOR_ENABLED` now enables restarts in Bash.
  The undocumented Bash/Zsh supervisor overrides for the check rate limit
  and maximum attempts are removed; use
  `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` and
  `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS`. The Fish-only
  `GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS` and
  `GPY_SUPERVISOR_LONG_BACKOFF_SECONDS` are now documented.
- Script plugin segments (`entry_type = "script"`): a public, documented
  contract in `docs/dev/plugins.md` with a `segments/<id>.toml` manifest,
  typed settings, enforced `min_gpy_version`, published JSON schemas, and
  user overrides in `config.toml` `[plugin.<segment>]` reachable by
  `gpy config get/set/list` as `plugin.<segment>.<key>`. Theme
  `[segments.<plugin>]` now accepts only style keys; free-form keys such as
  `context_color` are rejected in favour of `<variant>_bg_color` and
  `<variant>_text_color`, exported as
  `__gpy_segment_<token>_variant_<variant>_bg_color`/`_text_color` so
  variant `open`/`close` cannot overwrite the cap colours.
- The agent IPC no longer accepts the `hostname` and `username` ops or
  produces their `Hostname`/`Username` replies (#1077); both segments are
  bundled G plugins since #1003. Shells shipped since then never send them,
  and an older shell that does gets `{"error":...}` and falls back.

### Changed

- Numeric tuning variables (`GPY_RECONCILE_INTERVAL_SECS`, the hot-reload poll
  interval) now ignore surrounding whitespace like the watcher ones, and the
  language and font lookups find the home directory the way the rest of gpy
  does: through the passwd database when `HOME` is unset, never from a
  relative `HOME`.
- Library API: removed `MultiRepoWatcher::find_git_root` (use `git::find_git_dir_only`),
  `register_client` and `update_client` (merged as `attach_client`),
  `RepositoryState`'s `FromStr`, `ThemeManager::list_available_themes` and
  `default_theme_template`, `PaletteManager::default_palette_template`,
  `validate_config_key`, `validate_icon`, `get_language_color_from_theme`,
  and the `ThemeValidationResult`/`PaletteValidationResult` types
  (`commands::{theme,palette}::validate_*` return `ValidationReport`).
- Conflicting `--socket` and `GPY_AGENT_SOCKET_PATH` overrides now return an
  error instead of being silently accepted.
- Config validation errors name the exact field and the real rule.
  `gpy config set` reports a non-integer value as `Invalid whole number: <value>`
  instead of naming a Rust type (`u64 value`). A `git.skip_paths` entry such as
  `~root/x`, which was accepted but never matched, is now rejected, and
  `git.skip_paths[i]` / `ui.enabled_segments[i]` errors carry the full dotted
  path. A malformed `ui.theme` or `ui.palette` in `config.toml` now reads
  `invalid theme name '<name>': must not contain path separators, '..', or control characters`.
- A theme now renders the same on every load path. Without its own colours,
  a git state (`clean_*`, `dirty_*`, `ahead_behind_*`, `conflicts_*`,
  `untracked_only_*`, `in_progress_*`) takes the segment's `bg_color` and
  `text_color`, and a language takes `[segments.language]` `bg_color` and
  `text_color`. Before, a theme with no `[segments.git]` or
  `[segments.language]` table at all got built-in per-state and per-language
  colours, and one without `[segments.directory]` got a blue directory
  background (now black, as with an empty table). The shipped themes set
  these tables explicitly and look the same as before.
- `gpy-agent oneshot` falls back to the `default` theme when the configured
  theme fails to load, as the agent does. Before, it used a bare built-in
  theme with no segment formats and rendered the git, directory and language
  segments empty.
- `config/config.example.toml` is the one config template, shipped in the
  release archive and installed by `install-dev.fish`. It lists every
  setting, commented out at its built-in default, so a copy changes nothing
  and pins no key (before, `config/config.toml` and the archive's example each
  covered a different subset, with different git icons, and the dev install
  pinned the keys it listed). `[language.icons]` examples are
  commented too, so a fresh dev install shows language names until you enable
  them. The `text` theme no longer sets a Nerd Font duration icon, and
  `ui.show_icons` is described as also switching git icons and powerline
  caps.

### Fixed

- `gpy doctor` adds a warning with a `chmod -R go-w` hint when a plugin, a
  bundled one included, is dropped because a script or directory is group-
  or world-writable (for example a checkout made under umask 002). The shell
  test harnesses set explicit modes, so they pass under umask 002 (#1081).
- Zsh logs the same `GPY_DEBUG`/`GPY_VERBOSE` warning as Fish when
  `g_runner.bash` is missing, instead of disabling script segments silently
  (#1082).
- Script segments with different `timeout_ms` values in one prompt: each
  script now gets TERM at its own deadline and KILL 50 ms later, instead of
  waiting on the longest segment (Bash 5 runner, also used by Zsh and Fish;
  #1080).
- Theme validation rejects a plugin segment whose exported theme variables
  collide with a built-in theme export (for example `git_clean` with
  `bg_color` writing `__color_git_clean_bg`, or `status_fail` with `icon`
  writing `__icon_status_fail`), naming both; before, the plugin silently
  overwrote the built-in value (#1067).
- Serialization and UTF-8 decoding failures, debug-command socket I/O
  failures and an over-long socket path now report as JSON, I/O and
  configuration errors, and request-validation failures (paths, PIDs, message
  fields, message size) as invalid-operation errors. None of them say
  "IPC communication failed" any more, so their error prefix changes.
- `gpy theme import` maps a `[directory]` `read_only` symbol to
  `segments.directory.read_only_icon` instead of leaving the default lock.
- The `gpy config wizard` preview draws every language the prompt would (up to
  three) instead of only the first; the `docs/user/cli-reference.md` wizard
  sample shows the current layout.
- A `GPY_CONFIG_PATH`, `XDG_CONFIG_HOME` or `HOME` that is not valid UTF-8 is
  no longer rewritten with replacement characters, so the config file under it
  is found, loaded and saved.
- A `GPY_CONFIG_PATH`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_RUNTIME_DIR` or
  `HOME` that is not valid UTF-8 is no longer rewritten with replacement
  characters, so the config file, themes, palettes, plugins, caches and the
  runtime directory under it are found, loaded and saved.
- `gpy status` and the agent's own status query now read the agent's whole
  reply line; a status report longer than 4 KiB, or one split across reads,
  no longer fails to parse.
- `gpy restart` and `gpy doctor` no longer depend on the wording of
  `gpy-agent` output: restart learns whether an agent was running by pinging
  it, and doctor pings the agent itself. A failing `gpy start`, `stop` or
  `restart` now reports `command 'gpy-agent <subcommand>'` with its exit
  status.
- Bash and Zsh now fall back to a 100 ms duration threshold, as Fish and the
  exported theme do, instead of 2000 ms before a theme export runs.
- Fish hostname and username segments no longer print a `test` error on every
  prompt when the theme export has not set `show_always`.
- Fish and Zsh refuse an implausible `id -un` result (leading `-` or `.`,
  spaces, symbols) when resolving a blank `$HOME`, as Bash does.
- Fish completion offers theme, palette and segment names only in the
  argument's own position, as Bash and Zsh do.
- A client-supplied `virtual_env` is now validated like other paths: control
  characters, paths over 4096 bytes, and `..` components are rejected.
- `gpy theme new`, `gpy theme save`, `gpy palette import` and `gpy plugin new`
  create files exclusively, so a file created between the existence check and
  the write is reported as already existing instead of being overwritten.
  `gpy setup --shell fish` now fails when `gpy-dynamic.fish` is missing from
  the script tree instead of installing broken completions, `gpy setup`
  warns when it cannot ask about icons on `/dev/tty`, and `gpy doctor`
  reports config read and agent errors, inspects the segment file or script
  of every plugin entry type, and no longer flags comments or identifiers
  such as `node_count` as slow patterns.

- A theme whose plugin segment sections would set the same shell variable
  is now rejected, naming both fields. Segment names and variants both allow
  `_`, so segment `a` with an `open` cap and segment `a_open` with an `icon`
  both wrote `__gpy_segment_a_open_icon`, segment `a` with variant `x` and
  segment `a_variant_x` both wrote `__gpy_segment_a_variant_x_bg_color`, and
  segment `a` with variant `b_c` and segment `a_b` with variant `c` both wrote
  `__color_a_b_c_bg`; the later one silently won. The error reads
  `"segments.a.open.icon" and "segments.a_open.icon" both export shell
  variable "__gpy_segment_a_open_icon"`, and export skips the later section
  of a theme that was not validated. Rejected colour variant names, theme
  style keys and `gpy config set plugin.<segment>.<key>` values are now
  echoed escaped, so control characters in them no longer reach the terminal.
- Parallel `gpy config set` calls on different keys no longer lose updates.
  Each save re-read the file, edited it and renamed it into place with
  nothing serializing the three steps, so two started together could each
  write a file without the other's change and both exit 0. Saves now hold an
  exclusive lock on `config.toml.lock` beside the file (`flock` on Unix,
  `LockFileEx` on Windows), waiting up to 10 seconds.
- `gpy config wizard` saves a theme by writing a temp file and renaming it
  into place, as `config.toml` already was. Before, it truncated the theme
  file first, so a save that failed partway (disk full, crash) left a broken
  theme and the prompt fell back to the default. A symlinked theme file stays
  a symlink.
- `scripts/uninstall.sh` no longer treats end of input as confirmation. Run
  with stdin closed or redirected from `/dev/null`, it removed the config
  directory, with your themes and palettes, without asking; now it removes
  nothing and exits non-zero. Pass the new `--yes` to uninstall without a
  prompt.
- The Fish supervisor no longer kills an agent that missed one health check.
  A single ping that went unanswered within the prompt's 150 ms budget made it
  stop the agent, kill whatever held the socket and start a new one, dropping
  every cache. An agent that answers nothing is now restarted after two
  checks in a row, each allowed a second; one that is gone is still restarted
  at the next check.
- The protocol-mismatch hint, printed by the agent and by the Fish
  integration, now says to update gpy and re-run `gpy setup`. It used to
  point at `./install-dev.fish`, which only exists in a source checkout.
- Bash and Zsh escape prompt text (hostnames, usernames, icons, theme
  delimiters) in time that grows linearly with its length. The escape looped
  per character and Bash 3.2 also cost the length of the text per character,
  so a 4096-character value full of `$` took about half a second in
  `/bin/bash` on macOS. With default shell options the output is the same
  byte for byte; the Zsh escape also no longer garbles text when `ksh_arrays`
  or `glob_subst` is set.
- With `ui.show_icons = false`, the directory's read-only marker
  (`$read_only`) is `ro` instead of the 🔒 emoji.

### Added

- Plugin segments from v1 `.fish` files load in Fish only, and GPY now says
  so: `gpy plugin new` prints a one-line note, `validate` and `list` print it
  for `entry_type = "file"` plugins only, and `gpy enable <segment>` for a
  segment from such a plugin warns when a Bash or Zsh
  integration is installed that those prompts skip it (`gpy disable
  <segment>`, or use Fish). The developer docs and the INSTALL.md shell
  feature matrix carry the same statement. Script plugins
  (`entry_type = "script"`) are not affected. No change to prompt rendering.
- `scripts/gpy-dev.sh` (`just dev`) runs an unreleased build in an isolated
  fish shell: its own config, fish scripts, caches and agent socket, so an
  installed release is untouched and `exit` returns to it. Build the
  checkout, a `--ref`, a `--pr`, or use prebuilt binaries with `--bin`.
- `gpy config list [FILTER]` lists every config key with its access mode and
  description, or only the keys starting with `FILTER`. The command was
  documented but missing from the CLI, so it exited with `unrecognized
  subcommand`. A test now fails when `docs/user/cli-reference.md` names a
  `gpy` command the CLI does not have, and another when
  `docs/user/configuration-reference.md` and the config key registry disagree.
- Theme key `[segments.directory] read_only_icon` sets the directory
  format's `$read_only` marker (default 🔒).

### Changed

- `gpy plugin list` no longer prints a placeholder `root:` line for the
  built-in `gpy-core` plugin, which has no directory.
- `./install-dev.fish` now exits when a gpy outside `$HOME` (for example
  Homebrew's) is on `PATH`, since it would shadow that install and replace
  the Fish files `gpy setup` manages. Pass `--allow-shadow` to install anyway.
- `./install-dev.fish` no longer removes `conf.d/gpy_init.fish` or warns about
  `~/.config/gpy/core`, and the Fish integration no longer looks for
  `gpy-agent` in `~/.config/gpy/bin`; no installer writes those paths.
- CLI reports share one layout: underlined headings, details indented three
  spaces (`theme list`, `palette list`, `plugin list`, `config list`,
  `doctor`, `init`), and `theme list` and `palette list` print each entry's
  source. Error messages start lowercase and name invalid theme and palette
  names the same way. `gpy doctor` no longer says "this is fine", and the
  rc-file block that `gpy setup` writes is headed `GPY shell integration`
  (uninstall matches the marker lines, not the heading).
- The installers and `scripts/uninstall.sh` share one message style: plain
  `ok:`, `warn:` and `error:` prefixes, no glyphs or colour. `install.sh` now
  stages and tests a new binary before replacing the old one, as
  `install-oneline.sh` does, and its failure message names the step that
  failed instead of always blaming `~/.local/bin` permissions.
  `install-oneline.sh` also requires `fish/core/flags.fish` in the shell
  archive. `./install-dev.fish` stops the agent through `gpy-agent` and no
  longer deletes sockets from fixed paths.
- `gpy theme validate` and `gpy palette validate` choose their remediation
  hints from the kind of failure, so rewording an error no longer drops its
  hint. A palette file with a bad TOML value now reports
  `TOML parsing failed: ...` for named palettes too.
- `gpy config wizard`: the `?` help lists the browsing and filter keys plus
  only the highlighted segment's option keys, and the footers, detail-panel
  hints, help and `docs/user/cli-reference.md` key map now come from one key
  table, so they always name the keys the wizard handles. The reference now
  documents `/`, `?`, `[`/`]`, `v`, `:`, `m`, `b`, `a` and `t`.
- Fish: the debug helper is one function, `prompt-debug` (`prompt_debug` is
  gone), the segment-detect trace gated on `GPY_DEBUG_SEGMENTS` is removed
  (use `GPY_DEBUG=1`), and the agent supervisor lives in `core/supervisor.fish`.
- `theme export` no longer emits `__gpy_ui_prompt_icon`,
  `__gpy_ui_root_prompt_icon` or their `_icon_color` variants; no shell read
  them. `__icon_prompt`, `__icon_root_prompt`, `__prompt_color` and
  `__root_prompt_color` are unchanged.

## [0.6.0] - 2026-10-22

### Fixed

- `gpy-agent oneshot character --exit-code -1` is accepted and renders as a
  failed command; before, a negative value was rejected as an unknown flag
  unless written as `--exit-code=-1`.
- When the agent cannot parse a prompt request (for example an operation it
  does not know), it answers in the format the shell asked for, so the prompt
  shows an empty segment instead of printing a JSON error.
- With `ui.show_icons = false`, Nerd Font status icons (the default theme's
  ok/fail glyphs) are replaced by `ok`/`err`, and Nerd Font hostname and
  username icons are dropped, so they no longer show as boxes in plain fonts.
- The `username` segment now shows in SSH sessions too (`SSH_CONNECTION`,
  `SSH_CLIENT` or `SSH_TTY` set), as Starship's does, besides root, sudo and
  `show_always`.
- `gpy theme import` gives a `[username]` module without a `format`
  Starship's default `[$user]($style) in `; before, the trailing `in` was
  missing.

### Added

- `[segments.username]` gains `user_format`, `user_bg_color` (default
  `yellow`) and `user_text_color` (default `black`) for sessions not running as
  root, like Starship's `style_user`; `gpy theme import` maps `style_user` onto
  `user_format`.
- The `status` segment is drawn by the agent from a new optional
  `[segments.status] format`, like the other pills. The shells ask for it with
  a new `exit_status` IPC op (`gpy-agent oneshot status --exit-code N` shows
  the result). The bare `status` op is still the agent health check. A theme
  without the format keeps the shell-drawn pill.

### Changed

- Every pill now has one leading space inside it, in Fish, Bash and Zsh. The
  default theme gives `hostname`, `username` and `status` a `format` like the
  clock's, so they open with a space and no opening cap when first in the
  prompt. A theme without a `format` for one of them still gets the shell's
  local pill, which now carries the same leading space (before, it had none).
  With the agent down, disabled or answering with an error (an older agent
  that lacks `exit_status`), `hostname`, `username` and `status` fall back to
  that local pill instead of disappearing, so the SSH hostname and the
  root/sudo username still show.
- The `hostname` and `username` IPC requests accept `is_first`, and their
  `format` can use `$sep_open`, `$sep_gap`, `$sep_close`, `$style` and `$bg`
  like the clock's. Their separators are now chained like the clock's rather
  than terminal: a custom format using `$sep_close` now also draws the closing
  cap when the pill is not last, and `$sep_open`/`$sep_gap`, which used to
  resolve to nothing, now resolve. The Starship theme's formats use none of
  them and render as before. On the agent path `$symbol` is the segment's
  `icon` verbatim, so give an icon its own trailing space; the shell's local
  pill always puts a space between icon and text, so an icon without one
  (e.g. `"🌐"`) looks different on the two paths. The default theme leaves a
  local session without a doubled space.

## [0.5.1] - 2026-10-19

### Fixed

- A bare `gpy setup` works after installing with the curl installer or the
  release archive. Setup now keeps a copy of the shell scripts in
  `~/.local/share/gpy` when the installer passes their temporary location;
  before, it failed with "could not find GPY's fish files next to this
  binary". `scripts/uninstall.sh` removes the copy.

## [0.5.0] - 2026-10-19

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
- A `lang` request never waits on a language version probe beyond
  `agent.timeout_seconds`. A cold version is probed in the background, one
  probe per project at a time, and the version reaches idle Fish and Zsh
  prompts (and Bash at its next prompt) through the repaint doorbell
  (ADR-0009).
- Editing a language marker (`package.json`, `Cargo.toml`, ...) re-detects
  languages and re-probes versions on a background thread, so the agent's
  file-event handling no longer stalls for up to 10 s on a slow tool.
- Bash and Zsh build the flags tail, the forwarded virtualenv and the git and
  language requests without forking a subshell.
- When the prompt has nothing to show for a segment yet (a cold git cache, a
  directory not yet rendered in this shell), Fish, Bash and Zsh wait up to
  `GPY_IPC_COLD_TIMEOUT_MS` (default 500 ms, never below
  `GPY_IPC_TIMEOUT_MS`) for the agent instead of 150 ms. A heavily loaded
  machine then still draws those segments. Warm and stale renders keep the
  150 ms budget.

### Removed

- The hidden, deprecated `gpy theme use --apply-layout` alias; use `--force`.
  `gpy theme import --apply-layout` is unchanged.
- **Breaking:** `gpy-agent oneshot --format` and the IPC `format` field accept only `json`, `ansi`, `bash-prompt` and `zsh-prompt`. The unused `fish`, `fish-source` and `zsh` formats and the never-implemented `bash-source` and `zsh-source` are gone; no shipped shell sent them.
- **Breaking:** the unused IPC operations `theme` and `latency_stats` are removed; no shipped shell or CLI sent them.
- `scripts/update.fish`, which never detected a version change; run
  `just install` from the checkout.
- `uninstall.fish` and the `fisher-gpy.tar.gz` release asset are gone. Use
  `sh uninstall.sh`, which removes the Fish integration too (rc block, prompt
  symlink, prompt restore). GPY was never installable through Fisher: the
  plugin tree shipped no agent binary.
- Fish: the `gpy_lang_status` debug function, which printed settings nothing
  reads and called a function that no longer exists.

### Fixed

- The agent keeps accepting shell connections while it prunes dead clients or
  registers a shell's repository with the watcher, and it now stops its
  watchers even when installing its signal handlers fails.
- `gpy-agent status`, `gpy-agent stop` and `gpy debug paths` no longer create
  the runtime directory (`$XDG_RUNTIME_DIR/gpy` or its fallback); only
  `gpy-agent start` does.
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
- After an agent restart, Fish, Bash and Zsh drop their memoized character and
  directory renders when they re-register, so a config the restarted agent
  loaded (a directory truncation symbol, say) shows at once.
- At capacity, the agent forgets the least recently used repository's
  `prev_bg` render contexts instead of an arbitrary one.
- `scripts/bench.sh --ci` fails when `cargo bench` fails, and checks every
  budget before reporting an over-budget one.
- An idle Zsh shell no longer misses the agent's re-register ring when it
  arrives while the trap for the reload ring is still running. The shell used
  to go without live repaints until the agent re-nudged it a minute later. Zsh
  also acts on a pending ring at its next prompt now, as Bash does. A starting
  agent now rings each shell once for both `.reload` and `.reregister`, so
  Zsh shells still running older scripts re-register at once after an
  upgrade too.
- Upgrading through the installer for one shell refreshes the scripts of every
  other shell set up earlier, instead of leaving them on the old version.
  `gpy doctor` warns about a shell whose scripts an older `gpy` installed.
- Zsh startup no longer reads the whole of `$TMPDIR` to sweep stale render
  memo directories. Those directories now live in a private per-user parent,
  `$TMPDIR/gpy_relay_<uid>`, which is all the sweep reads. A `$TMPDIR` holding
  many entries from other programs used to add a second or more to every Zsh
  start, and over ten seconds on a loaded machine.
- Finding a directory's language project root no longer lists every entry of
  each ancestor directory. Marker files are looked up by name first, and
  other spellings only within a directory's first 1024 entries, so a prompt
  under a directory with many files (a crowded `$TMPDIR` or downloads
  folder) no longer times out the language segment.
- A language request whose version probe finished in time no longer fails
  when the version's cache entry is already gone again (expired, or read as
  stale after the system clock stepped back); the language shows without a
  version.

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

### Upgrading from 0.4.0

- Run the installer as before. It restarts the agent on the new version and
  refreshes the scripts of every shell set up earlier, not only the one it
  detects. Shells already open keep working against the new agent and
  re-register with it at once; new shells load the new scripts.
- `gpy doctor` warns about any shell whose scripts an older `gpy` installed;
  `gpy setup --shell <shell>` refreshes them.
- A `GPY_AGENT_SOCKET_PATH` directly in a shared directory such as `/tmp` is
  now refused; point it into a private directory.

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

[Unreleased]: https://github.com/jpease/gpy/compare/v0.7.1...HEAD
[0.7.1]: https://github.com/jpease/gpy/releases/tag/v0.7.1
[0.6.0]: https://github.com/jpease/gpy/releases/tag/v0.6.0
[0.5.1]: https://github.com/jpease/gpy/releases/tag/v0.5.1
[0.5.0]: https://github.com/jpease/gpy/releases/tag/v0.5.0
[0.4.0]: https://github.com/jpease/gpy/releases/tag/v0.4.0
