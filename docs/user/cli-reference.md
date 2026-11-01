# GPY Command-Line Interface Reference

Complete reference for the `gpy` command-line tool.

## Overview

`gpy` is the user-facing command-line interface for managing the GPY prompt for Bash, Fish and Zsh. It controls the agent, themes, segments and configuration.

A second binary, `gpy-agent`, is the daemon itself. It also carries the few subcommands the shell integrations call (`oneshot`, `theme export`, `config reload`); they are listed under [The `gpy-agent` binary](#the-gpy-agent-binary) below.

Checks and validations mark each result with a colored ✓, ✗ or ! on a terminal, and with `ok:`, `FAIL:` or `warn:` when output is not a terminal, `NO_COLOR` is set, `TERM=dumb`, or `ui.show_icons = false`. Other commands print plain text.

## Quick Reference

```bash
# Agent lifecycle
gpy start                    # Start the background agent
gpy stop                     # Stop the background agent
gpy restart                  # Restart the agent
gpy status                   # Show agent status

# Theme management
gpy theme list               # List available themes
gpy theme use <name>         # Switch to a theme
gpy theme show               # Show current theme
gpy theme new <name> [--from <base>] [--full]  # Create a theme that extends <base> (default: default)
gpy theme save [name] [--full]  # Save the active theme as a theme file that extends it
gpy theme validate [name-or-path]  # Validate active theme, name, or file path
gpy theme diff <name>        # Compare your copy of a builtin theme with the builtin
gpy theme reset <name>       # Move your copy of a builtin theme to .bak
gpy theme migrate <name>     # Rewrite keys removed by breaking changes (backs up first)
gpy theme rebase <name> [--onto <base>] [--write] [--unset-missing]  # Turn a modified full copy into an overlay

# Palette management
gpy palette list                 # List available palettes
gpy palette use <name>           # Switch to a palette
gpy palette show                 # Show current palette
gpy palette validate [name-or-path]  # Validate active palette, name, or file path
gpy palette import <file>        # Import a base16/base24 scheme as a palette

# Plugin management
gpy plugin list                              # List discovered plugins
gpy plugin new <id> [--segment <segment>]    # Create plugin scaffold
gpy plugin install <dir> [--force]           # Copy a plugin into your plugins directory
gpy plugin remove <id>                       # Back up a user plugin and disable its segments
gpy plugin validate <id-or-path>             # Validate plugin files and manifest

# Segment management
gpy enable <segment>         # Enable a segment
gpy disable <segment>        # Disable a segment
gpy segments                 # List all segments

# Configuration
gpy config show [section]    # Show configuration (sections: agent, git, language, ui, plugin)
gpy config list [filter]     # List config keys, optionally by prefix
gpy config get <key>         # Get a config value
gpy config set <key> <value> # Set a config value
gpy config open              # Open the config file in your editor
gpy config wizard            # Launch the interactive configuration wizard

# Diagnostics
gpy doctor                   # Run health checks
gpy completions <shell>      # Generate shell completions

# Install
gpy setup [--shell <shell>]  # Wire GPY into Fish, Zsh or Bash
gpy setup --redetect-icons   # Re-run Nerd Font detection for ui.show_icons
gpy uninstall [--yes] [--purge-config]  # Remove GPY's integration, rc blocks, cache and binaries
```

---

## Commands

### Agent Lifecycle

#### `gpy start`

Start the background agent process.

The agent runs as a daemon and handles git status detection, language detection, and live prompt updates.

**Example:**
```bash
gpy start
```

**Output** (on stdout, one line; `gpy-agent start --quiet` prints nothing on
success):
```
Started agent (pid 4321).
```

The command returns once the daemon answers on its socket; `gpy status`
then reports `state: running` with that PID. Starting is idempotent: with a
matching agent already running it prints `Agent already running.`, exits `0`
and forks no second daemon, and with an older agent running it replaces it and
prints `Replaced agent <old> with <new> (pid N).` so an upgrade never leaves a
stale daemon serving new shells. It also replaces an agent that speaks a
different protocol, even a newer one: shells built for this protocol cannot
use it. A newer agent on the same protocol is left alone
(`Agent <running> already running; it is newer than this gpy-agent (<this>) and
was left in place.`). An agent that is running but too busy to answer (it
closes new connections without a reply) is left alone too: `gpy-agent start`
prints `Agent at <socket> is too busy to answer; leaving it running.` on
stderr and exits `0`. `agent.enabled = false` is honored even when other keys
in `config.toml` are invalid: `start` prints `Agent disabled in config
(agent.enabled = false); not started.` on stderr, binds no socket, and exits
`6`. If the new daemon exits or does not answer within 5 seconds, `gpy start`
(and `gpy-agent start`) exits `1` with an `Error:` line; when the daemon fails
before binding its socket, the line carries the daemon's own error, such as a
cache directory it could not create and why. When `start` cannot tell whether
an agent already owns the socket, it leaves the socket alone and exits `1`.
The daemon runs in `/`, so it never keeps the
directory you started it from (or its volume) busy; a relative `--socket`,
`GPY_AGENT_SOCKET_PATH`, `GPY_CONFIG_PATH` or `GPY_DEBUG_LOG` still
resolves against that directory.

**Exit Codes:** `0` agent running afterwards (started, replaced an older one,
already running, busy, or a newer one left in place), `1` error, `6` disabled
by `agent.enabled = false`, `2` usage error.

The socket's directory, default or `GPY_AGENT_SOCKET_PATH`/`--socket`, must
be a real directory you own. `gpy start` exits `1` naming the directory if it
is a symlink or belongs to another user, so a socket directly in `/tmp` is
refused. If the directory is yours but group- or world-writable, GPY's default
runtime directory is set to `0700` and used; the directory of an explicit
socket path is refused and left unchanged (`chmod 700` it, or pick a private
directory).

---

#### `gpy stop`

Stop the running background agent.

Sends a graceful shutdown signal to the agent.

**Example:**
```bash
gpy stop
```

**Output** (stdout):
```
Stopped agent (pid 4321).
```

`gpy-agent stop --quiet` prints nothing on success. An agent that reports no
PID prints `Stopped agent.`.

Stopping is idempotent: with no agent running it prints `Agent not running.`
and still exits `0`.

A socket file with nothing listening is a leftover, not a running agent: `stop`
removes it, prints `Agent not running; removed stale socket <path>.` and exits
`0`. If an agent is there but cannot be stopped (it does not answer the
shutdown request, or still answers after the shutdown wait), `stop` prints one
`Error:` line on stderr naming the socket (and `kill <pid>` when the agent
reported its PID) and exits `1`.

**Exit Codes:** `0` agent stopped afterwards (including not running), `1` the
agent still answers or did not answer the shutdown, `2` usage error. `stop`
never exits `6`: a disabled agent that is still running must be stoppable.

---

#### `gpy restart`

Restart the background agent.

Runs `gpy-agent restart`, which stops the agent and then starts it. A failed
stop (for example an agent that never answers) is printed as a `warn:` line on
stderr and does not abort, because `start` evicts an agent that does not
answer.

**Example:**
```bash
gpy restart
```

**Output** (stdout; the stop line is `Agent not running.` when nothing ran):
```
Stopped agent (pid 4321).
Started agent (pid 4388).
```

**Exit Codes:** as `gpy start`: `0` agent running afterwards, `1` error, `6`
disabled by `agent.enabled = false`, `2` usage error.

---

#### `gpy status`

Display agent status. `gpy status` runs `gpy-agent status`, so both print the
same fixed lowercase `key: value` lines on stdout, in this order, and nothing
else.

**Example:**
```bash
gpy status
```

**Output:**
```
state: running
pid: 41235
version: 0.9.0
protocol: 2
socket: /run/user/1000/gpy/gpy.sock
config: /home/user/.config/gpy/config.toml
watched_repos: 1
registered_clients: 1
cache_entries: 2
```

`state` is one of `running`, `busy`, `not-running` and `not-answering`, and it
always agrees with the exit code:

| Situation | `state` | Exit |
|---|---|---|
| The agent answered the status request | `running` | `0` |
| The agent accepted the connection and dropped it unanswered (it is shedding load) | `busy` | `0` |
| No socket file, or the socket refuses connections (a stale file) | `not-running` | `3` |
| The socket accepts but no valid reply arrives (wedged), or the status could not be determined | `not-answering` | `4` |

`pid`, `version`, `protocol`, `watched_repos`, `registered_clients` and
`cache_entries` are printed only for a `running` agent (`pid` is omitted when
an older agent did not report it). `config` is the active config file, or
`none (built-in defaults)`. `error` is the reason for a `not-answering` agent,
for a `busy` agent (`agent closed the connection without a reply`) and for a
`not-running` state caused by a socket file that refused the connection. A
status that cannot even resolve the socket path prints one `Error:` line, no
`key: value` lines, and exits `4`.

Scripts test liveness with the exit code and read values with
`awk -F': '`. A protocol mismatch adds one `warn:` line on stderr.
`gpy-agent status` follows the same contract.

**Exit Codes:** `0` running or busy, `3` not running, `4` not answering or
unknown, `2` usage error. `gpy status` never exits `1`: when `gpy-agent` cannot
be run it prints one `Error:` line and exits `4`.

---

### Theme Management

#### `gpy theme list`

List all available themes with the active theme marked.

Scans both built-in themes (in the installation directory) and user themes (in `~/.config/gpy/themes/`).

**Example:**
```bash
gpy theme list
```

**Output:**
```
Available Themes:
=================

   default * [builtin]
   text [builtin]
```

The `*` marker indicates the currently active theme. `[builtin]`/`[user]`/`[plugin:<id>]` shows the theme's source.

An overlay (a user theme with `extends`) also shows its base, e.g.
`starship [user, extends builtin starship]`.

---

#### `gpy theme use <name>`

Switch to a different theme.

Validates that the candidate theme's segment format templates render successfully
against the prospective active palette before persisting any changes. If template
rendering fails (unknown color, unbalanced syntax), activation aborts with exit
code 1, leaving the active configuration byte-for-byte unchanged.

On successful validation, updates `config.toml` and reloads the running agent.
**Arguments:**
- `<name>` - Theme name (without `.toml` extension)

**Example:**
```bash
gpy theme use text
```

**Options:**
- `--force` - Also apply the theme's recommended layout (`[ui.recommended]`:
  segment order, directory display, language icons, ...) to `config.toml`,
  preserving any setting you configured explicitly. Without it, a theme that
  carries recommendations prints a `Re-run with --force to apply
  recommended ...` hint and leaves your layout alone.

**Output:**
```
Switched to 'text' theme
```

That is the whole output when the running agent acknowledged the reload, and
when no agent is running (it reads the config when it starts). When a running
agent rejected the config or did not answer, one more line follows:

```
! Agent not reloaded: <reason>
```

Every command that edits `config.toml` (`theme use`, `palette use`,
`enable`, `disable`, `config set`) ends the same way: silent
on a confirmed reload or with no agent, one warning otherwise. Switching
*themes* currently prints the warning even with a live agent on macOS, because the switch re-arms
its theme-file watcher for longer than the CLI waits; the reload still
happens a few seconds later.

---

#### `gpy theme import <file>`

Import a `starship.toml` into a matching GPY palette and prompt theme pair.
The migration guide is in [Migrating from Starship](migrating-from-starship.md).

**Arguments:**
- `<file>` - Path to the source `starship.toml`

**Options:**
- `--name <name>` - Base name for the emitted palette and theme (default:
  the file stem)
- `--force` - Overwrite existing artifacts of the same name, or shadow a
  builtin/plugin theme or palette of that name (without it, both are refused)
- `--stdout` - Print both artifacts instead of writing files
- `--apply-layout` - Also write the derived `enabled_segments` into the
  active config

**Example:**
```bash
gpy theme import ~/.config/starship.toml --name my-prompt --apply-layout
```

---

#### `gpy theme show`

Display the name of the currently active theme.

**Example:**
```bash
gpy theme show
```

**Output:**
```
default
```

---

#### `gpy theme new <name> [--from <base>] [--full]`

Create a new theme as an overlay of an existing theme: a file that holds `extends = "<base>"` and, once you edit it, only the keys you change. Everything else comes from the base, including its later updates.

Destination names must satisfy the safe configuration name invariant: non-empty,
non-whitespace, containing no path separators (`/`, `\`), `..`, or control characters.
An invalid destination name is rejected before creating files or directories.

Creates `~/.config/gpy/themes/<name>.toml`. Without `--from`, the base is `default`, resolved like `gpy theme use default`, so a user `default.toml` is picked up if you have one. With `--from <base>`, the base is that theme (a builtin, user, or plugin theme name). The new file is loaded before it is written; if it would not load (an unknown base, an `extends` cycle), nothing is written. The output names the base it resolved to and, when the base is a plugin theme, the plugin the new theme depends on: removing or disabling that plugin leaves the theme unable to load.

With `--full`, the file is a full, independent copy of the base instead: the base's file verbatim (comments kept) when the base is a full theme, or the merged result when the base is itself an overlay. A full copy no longer receives changes to its base.

**Arguments:**
- `<name>` - New theme name (non-empty safe config name)

**Options:**
- `--from <base>` - Theme the new theme extends (default: `default`)
- `--full` - Write a full, independent copy of the base instead of an overlay

**Example:**
```bash
gpy theme new mytheme

# Build on the starship preset instead of the default theme
gpy theme new mytheme --from starship

# A frozen copy of the starship preset, independent of later releases
gpy theme new mytheme --from starship --full
```

**Output:**
```
Created new theme: /home/user/.config/gpy/themes/mytheme.toml
Extends: starship (builtin)
Edit: /home/user/.config/gpy/themes/mytheme.toml
Activate with: gpy theme use mytheme
```

`~/.config/gpy/themes/mytheme.toml`:
```toml
extends = "starship"
```

---

#### `gpy theme save [name] [--full] [--yes]`

Save the active theme as a theme file that extends it.

With `<name>`, writes `~/.config/gpy/themes/<name>.toml` with `extends = "<active theme>"`: created without prompting if no theme file by that name exists yet, or replaced after a `[y/N]` confirmation if it does (the old file is kept as `.bak`).

Without `<name>`, or with the active theme's own name:
- When the active theme is a builtin or plugin theme, it writes a self-named overlay, e.g. `starship.toml` with `extends = "starship"`, after a `[y/N]` confirmation. The theme keeps its name and still receives updates to the builtin or plugin theme; add your changes to the file.
- When the active theme already is a file in the themes directory (an overlay or a full theme), nothing is written and the command says so: that file already is your theme. Edit it directly, or save it under another name.

The new file is loaded before anything is written; if it would not load (for example `gpy theme save starship` while the active theme `mine` extends `starship`, which would be an `extends` cycle), nothing is written. The output names the theme the file extends and any plugin it depends on.

With `--full`, the file is a full, independent copy of the active theme instead: its file verbatim when it is a full theme, or the merged result when it is an overlay. `--full` to the active theme's own file is refused like the plain form.

Declining the prompt (anything other than `y`/`yes`) exits 1 with `theme save cancelled; nothing was written`. When input ends without an answer (for example `</dev/null`), it exits 1 with `theme save needs confirmation to write <path>; pass --yes`. Piped answers (`echo y | gpy theme save`) work. `--yes` skips the question.

**Arguments:**
- `[name]` - Theme name to save as (default: the active theme's own name; asks before writing unless `--yes`)

**Options:**
- `--full` - Write a full, independent copy of the active theme instead of an overlay
- `-y`, `--yes` - Skip the confirmation question

**Example:**
```bash
# With a builtin active: make it an editable theme file that keeps its name
gpy theme save

# Save a theme that extends the active one under a new name
gpy theme save mytheme

# Freeze the active theme's current look under a new name
gpy theme save mytheme --full
```

**Output:**
```
Saved theme: /home/user/.config/gpy/themes/mytheme.toml
Extends: starship (builtin)
Edit: /home/user/.config/gpy/themes/mytheme.toml
Activate with: gpy theme use mytheme
```

---

#### `gpy theme validate [name-or-path]`

Validate a theme and print actionable diagnostics.

Validates theme schema fields, icon definitions, and segment format templates
across all nine supported agent-rendered segments (`git`, `language`, `directory`,
`duration`, `character`, `clock`, `hostname`, `username`, `status`) against the active palette,
verifying that all color tokens resolve to valid ANSI colors or defined palette roles.

If `target` is omitted, validates the currently active theme from config.
If `target` is provided, it can be either:
- a theme name (for discovered themes), or
- a direct path to a theme `.toml` file.
- `[target]` - Optional theme name or file path

Warnings (validation still passes): `unset` in a file without `extends`
(it does nothing), and an `unset` path the base theme lacks (ignored at load).
A file in a plugin's `themes/` directory may extend only a builtin theme.

**Examples:**
```bash
# Validate active theme from config
gpy theme validate

# Validate a named theme
gpy theme validate default

# Validate a local theme file directly
gpy theme validate ~/.config/gpy/themes/mytheme.toml
```

**Success output:**
```
✓ Theme validation passed
   target: default
   source: active config theme
```

**Failure output (example):**
```
✗ theme validate failed
   reason: Configuration error: Invalid color 'not-a-color' for field ui.prompt_color in theme /home/user/.config/gpy/themes/mytheme.toml
   remediation:
   - Use named colors, ANSI 0-255, or #RRGGBB values.
```

---

#### `gpy theme diff <name>`

Show how your copy of a builtin theme differs from the builtin that ships with this version of GPY.

A file at `~/.config/gpy/themes/<name>.toml` completely replaces the builtin theme of the same name, so it stops receiving upstream changes and can break after an upgrade. For a full copy the output is a unified diff: `-` lines exist only in the builtin, `+` lines only in your copy. For an overlay (a user theme with `extends`) it lists the keys whose effective value the overlay changes. The command exits 0 when `<name>` is a builtin or an overlay; it says so when there is no copy to compare or the copy is identical.

**Arguments:**
- `<name>` - Theme with a user file: your copy of a builtin, or one of your overlay themes (see `gpy theme list`)

**Example:**
```bash
gpy theme diff starship
```

---

#### `gpy theme reset <name>`

Move your copy of a builtin theme aside so the builtin takes over again.

The file `~/.config/gpy/themes/<name>.toml` is renamed to `<name>.toml.bak`; an existing backup is never overwritten, so later resets use `.bak.1`, `.bak.2`, and so on. When `<name>` is the active theme the running agent is reloaded. `reset` fails when there is no user file to move.

**Arguments:**
- `<name>` - Theme with a user file: your copy of a builtin, or one of your overlay themes (see `gpy theme list`)

**Example:**
```bash
gpy theme reset starship
```

**Output:**
```
Moved /home/user/.config/gpy/themes/starship.toml to /home/user/.config/gpy/themes/starship.toml.bak
The builtin theme 'starship' is in use again.
```

---

#### `gpy theme migrate <name>`

Rewrite a theme file in `~/.config/gpy/themes/` for keys removed by breaking changes, keeping your other customizations and comments.

Rules are grouped by the release that removed the keys. The 0.7 rules (the `username` and `hostname` segments became bundled plugins):

- `format` (and `user_format`) are removed from `[segments.username]` and `[segments.hostname]`.
- `show_always` and `trim_at` move to `config.toml` `[plugin.username]` / `[plugin.hostname]`. A value already set there is kept.
- `icon` of `[segments.hostname]` is renamed `ssh_icon` unless `ssh_icon` is already set.

Before anything is written, the theme file and `config.toml` are copied to `.bak` (`.bak.1`, ... when a backup exists). Every rewrite is printed, and the result is checked like `gpy theme validate`; if the theme still does not load, the command says why, exits non-zero and leaves the backup in place. A theme with nothing to rewrite prints `Nothing to migrate` and exits 0, so running it twice is safe. `migrate` accepts any theme file in the themes directory: an overlay, a full copy made with `gpy theme new --full`, or a theme of your own.

**Arguments:**
- `<name>` - Name of a theme file in the themes directory

**Example:**
```bash
gpy theme migrate starship
```

---

#### `gpy theme rebase <name> [--onto <base>] [--write] [--unset-missing]`

Rewrite a modified full copy of a theme as an overlay: a file with `extends = "<base>"` that holds only the keys you changed, so it receives every later fix to the keys it does not set.

The copy and the base are compared by their effective values, not their text, so a copy written by the wizard of an older GPY (every default spelled out, language keys as nested tables) compares like a hand-written one. A key whose value is only the default its segment applies anyway (for example `show_seconds = false`) counts as unset. Each key your file sets is dropped when it equals the base and kept otherwise, in your file's spelling and with its comments. Keys GPY ignores are dropped and listed.

Keys the base has but your copy lacks are inherited from the base afterwards, because the copy cannot tell a key added upstream (which you should now receive) from one you deleted. They are listed at the highest level that is missing (a whole `[segments.hostname]` table, or `segments.clock.open`, not each key inside). `--unset-missing` writes one `unset` entry for each of them instead, for keys you removed on purpose. Without it, `rebase` never writes `unset`.

The command prints a report and the overlay and writes nothing unless `--write` is given. Before printing, it checks that the overlay loads and gives the same theme as your copy, apart from the inherited keys; otherwise it fails and writes nothing. `--write` copies the file to `.bak` (`.bak.1`, ... when a backup exists) and then replaces it in one step, so a failed write leaves your copy in place. A running agent picks up the change like any other edit of the theme file.

A copy that does not load, such as one with keys removed by a breaking change, is refused: run `gpy theme migrate <name>` first. A file that already has `extends` is refused too.

**Arguments:**
- `<name>` - Name of a full theme file in the themes directory

**Options:**
- `--onto <base>` - Theme the overlay extends. Defaults to the plugin or builtin theme with the same name, the one your copy shadows; for a copy with no such theme it is required, and the error names the closest builtin.
- `--write` - Write the overlay over the file, after copying it to `.bak`
- `--unset-missing` - Unset the inherited keys instead of inheriting them

**Example:**
```bash
gpy theme rebase starship           # show the overlay and what it inherits
gpy theme rebase starship --write   # replace the copy, keeping starship.toml.bak
gpy theme rebase mine --onto default --write
```

**Output:**
```
Rebasing /home/user/.config/gpy/themes/starship.toml onto starship (builtin)
   Kept, as different from the base: segments.clock.time_format
   Dropped as equal to the base: 212 keys

extends = "starship"

[segments.clock]
time_format = "24"

Nothing written. Run `gpy theme rebase starship --write` to replace /home/user/.config/gpy/themes/starship.toml (a copy is kept as .bak).
```

---

### Palette Management

#### `gpy palette list`

List all available palettes with the active palette marked.

Scans both built-in palettes and user palettes (in `~/.config/gpy/palettes/`).

**Example:**
```bash
gpy palette list
```

**Output:**
```
Available Palettes:
===================

   catppuccin-frappe [builtin]
   catppuccin-latte [builtin]
   catppuccin-macchiato * [builtin]
   catppuccin-mocha [builtin]
   default [builtin]
   gruvbox-dark-medium [builtin]
   nord [builtin]
   starship [builtin]
```

The `*` marker indicates the currently active palette. `[builtin]`/`[user]`/`[plugin:<id>]` shows the palette's source.

---

#### `gpy palette use <name>`

Switch to a different palette.

Updates the configuration and reloads the running agent to apply changes immediately. Only the named-color set changes — the active theme's structure (segments, templates, delimiters) is unaffected.

**Arguments:**
- `<name>` - Palette name (without `.toml` extension)

**Example:**
```bash
gpy palette use nord
```

**Output:**
```
Switched to 'nord' palette
```

---

#### `gpy palette show`

Display the name of the currently active palette.

**Example:**
```bash
gpy palette show
```

**Output:**
```
default
```

---

#### `gpy palette validate [name-or-path]`

Validate a palette and print actionable diagnostics.

If `target` is omitted, validates the currently active palette from config.
If `target` is provided, it can be either:
- a palette name (for discovered palettes), or
- a direct path to a palette `.toml` file.

**Arguments:**
- `[target]` - Optional palette name or file path

**Examples:**
```bash
# Validate active palette from config
gpy palette validate

# Validate a named palette
gpy palette validate nord

# Validate a local palette file directly
gpy palette validate ~/.config/gpy/palettes/mine.toml
```

**Success output:**
```
✓ Palette validation passed
   target: nord
   source: named palette
```

**Failure output (example):**
```
✗ palette validate failed
   reason: Configuration error: No palette named 'no-such-palette' found (checked user path and builtins)
   remediation:
   - Confirm the palette is in ~/.config/gpy/palettes or is a known builtin.
```

---

#### `gpy palette import <file>`

Import a base16/base24 scheme file as a new GPY palette.

Accepts both the modern nested layout (`system:`, `name:`, `variant:`, then a `palette:` block of `baseNN: "hex"` entries) and the legacy flat layout (`scheme:`/`name:` plus top-level `baseNN:` keys). `base00`–`base0F` cover base16; base24 adds `base10`–`base17`.

**Arguments:**
- `<file>` - Path to a base16/base24 scheme YAML file

**Flags:**
- `--name <name>` - Override the palette name (defaults to the scheme's `slug` field, else its `name`/`scheme` field, falling back to `imported` if the scheme declares none; the name is lowercased, non-letter/digit runs become `-`, and non-ASCII letters are kept)
- `--force` - Overwrite an existing palette of the same name, or shadow a builtin palette of that name; without it, import fails if the destination already exists or the name belongs to a builtin palette

**Example:**
```bash
gpy palette import ~/Downloads/gruvbox-dark-hard.yaml --name gruvbox-hard
```

**Output:**
```
Imported palette 'gruvbox-hard' → /home/user/.config/gpy/palettes/gruvbox-hard.toml
```

The imported palette is written to `~/.config/gpy/palettes/<name>.toml`. If the derived name would be unsafe (e.g. contains path separators), the import fails with an error rather than writing outside the palettes directory.

---

### Segment Management

#### `gpy enable <segment>`

Enable a prompt segment.

A segment renders when it is listed in `ui.enabled_segments`; `git` and `language` additionally need their feature flag (`git.enabled` / `language.enabled`). `gpy enable` adds the segment to `ui.enabled_segments` if it is missing, and for `git` and `language` also sets the feature flag to `true`, in a single config save.

**Valid Segments:**
- `git` - Git repository status
- `lang` / `language` - Programming language detection
- `clock` - Current time
- `duration` - Command execution time
- `directory` - Current working directory
- `status` - Previous command exit status pill (✔/✖); coloured by `segments.status.ok_*`/`fail_*` and drawn by the agent through `segments.status.format`
- `username` - Current user name; a bundled script plugin. Shown only as root, under sudo or over SSH unless `plugin.username.show_always` is set in `config.toml` (opt-in). The plugin prints the variant `root` as root, styled with the theme's `root_bg_color`/`root_text_color` (and `root_icon`), like Starship's `style_root`; other sessions use `bg_color`/`text_color`
- `hostname` - Machine hostname; a bundled script plugin. Shown only over SSH unless `plugin.hostname.show_always` is set in `config.toml` (opt-in), cut at `plugin.hostname.trim_at`. Its `ssh` variant icon (`ssh_icon` in `[segments.hostname]`, Starship's `ssh_symbol`) shows only in SSH sessions

**Example:**
```bash
gpy enable git
gpy enable language
gpy enable clock
```

**Output:**
```
Enabled git segment
```

---

#### `gpy disable <segment>`

Disable a prompt segment.

Removes the segment from the prompt display without deleting any configuration. Most segments are removed from `ui.enabled_segments`. For `git` and `language`, only the feature flag (`git.enabled` / `language.enabled`) is set to `false`; the list entry stays, so a later `gpy enable` puts the segment back in its original position.

**Example:**
```bash
gpy disable clock
gpy disable git
```

**Output:**
```
Disabled clock segment
```

---

#### `gpy segments`

List all available segments and their current status.

Shows which segments the prompt will actually render (`x`) and which it won't ( ). `git` and `language` show `x` only when they are listed in `ui.enabled_segments` **and** their feature flag is on; every other segment shows `x` when it is listed. `gpy config wizard` starts its segment checkboxes from the same rule.

Each row also names the segment's owner: `gpy-core` for the built-in segments, or the plugin id with `(bundled)` or `(user)` for a plugin's segments. `default` marks a segment that a fresh config enables (`ui.enabled_segments` by default).

**Example:**
```bash
gpy segments
```

**Output:**
```
Prompt Segments:
================

[x] clock     gpy-core            default
[x] duration  gpy-core            default
[x] language  gpy-core            default
[x] directory gpy-core            default
[x] git       gpy-core            default
[ ] status    gpy-core
[ ] username  username (bundled)
[ ] hostname  hostname (bundled)
```

---

### Configuration Management

#### `gpy config show [section]`

Display configuration values.

Without a section argument, shows all configuration. With a section argument, shows only that section; any other value is a usage error (exit `2`) that lists the valid sections. Every field is printed, including the `[git.icons]` and `[language.icons]` tables; the output is valid TOML (section headings are comments).

**Valid Sections:**
- `agent` - Agent process settings
- `git` - Git detection settings
- `language` - Language detection settings
- `ui` - User interface settings
- `plugin` - Script plugin segment overrides (`[plugin.<segment>]`)

**Example:**
```bash
# Show all configuration
gpy config show

# Show only git configuration
gpy config show git
```

**Output:**
```
# Git Configuration:
# ==================

[git]
enabled = true
icon_set = "unicode"
max_ahead_behind = 100
max_branch_length = 0
show_upstream = true
skip_paths = []
stash_enabled = true
timeout_seconds = 10
watch_worktree = true

[git.icons]
ahead = "↑"
behind = "↓"
conflicts = "✖"
detached = "➦"
in_progress = "↻"
staged = "✚"
stash = "≡"
unstaged = "✱"
untracked = "?"
```

---

#### `gpy config list [filter]`

List every configuration key with its access mode and description.

After the core keys it lists the `language.icons.<lang>` family (one row for the settable `[language.icons]` entries, e.g. `language.icons.rust`) and the `plugin.<segment>.<key>` overrides that installed script plugin segments accept (`ttl_secs`, `timeout_ms` and each declared setting, with its type, constraints and default).

With a filter argument, lists only keys that start with it. The filter is a plain prefix, so `git.icons` matches every `git.icons.*` key. A filter that matches nothing prints a notice and still exits 0. Use `gpy config get <key>` to read a value and `gpy config set <key> <value>` to change it; the [configuration reference](configuration-reference.md) documents what each key accepts.

**Example:**
```bash
# List every key
gpy config list

# List only the git icon keys
gpy config list git.icons
```

**Output:**
```text
Available configuration keys:
=============================

   git.icons.ahead [read-write]
      Ahead indicator

   git.icons.behind [read-write]
      Behind indicator

   ...

Total: 9 keys
```

---

#### `gpy config get <key>`

Get a single configuration value.

Uses dot-notation to specify the configuration key.

**Example:**
```bash
gpy config get git.enabled
gpy config get ui.theme
gpy config get language.show_versions
```

**Output:**
```
true
```

---

#### `gpy config set <key> <value>`

Set a configuration value.

Updates the configuration file and reloads the running agent.

**Common Keys:**
- `agent.enabled` - Enable/disable background agent
- `agent.live_updates` - Enable/disable live prompt updates
- `git.enabled` - Enable/disable git detection
- `git.show_upstream` - Show/hide ahead/behind indicators
- `language.enabled` - Enable/disable language detection
- `language.show_versions` - Show/hide version numbers
- `ui.theme` - Active theme name
- `ui.show_icons` - Show/hide Nerd Font icons
- `ui.transient_prompt` - Collapse submitted prompts to time, prompt symbol and command (Fish 4.2+, Zsh)
- `ui.directory.display` - Directory segment display mode: `basename`, `abbreviated`, `truncated`, or `full`
- `ui.directory.truncation_length` - Path components kept when `display = "truncated"` (default `3`)
- `ui.directory.truncation_symbol` - Prefix shown before a truncated path, e.g. `…/` (default empty)
- `ui.directory.max_length` - Maximum displayed length for the rendered directory string
- `language.display` - Language segment display: `icon` or `text`
- `language.filter` - Languages to show: `all`, `primary`, or top N (e.g. `3`)
- `language.detection_mode` - Detection strategy: `markers` (default), `content`, or `hybrid`
- `language.icons.<lang>` - Icon for one language, e.g. `language.icons.rust`; aliases such as `rs` resolve to the same entry
- `git.max_ahead_behind` - Maximum ahead/behind count to report (`0` = unlimited)
- `git.stash_enabled` - Enable/disable the stash count
- `git.icon_set` - Glyph set for stash/detached/in-progress icons: `unicode` or `nerd_font`
- `git.icons.<name>` - Git indicator icon; `<name>` is one of `ahead`, `behind`, `staged`, `unstaged`, `untracked`, `conflicts`, `stash`, `detached`, `in_progress`
- `ui.palette` - Named color palette; must exist (checked before writing)

List-valued keys (`git.skip_paths`, `language.enabled_languages`, `ui.enabled_segments`) can be read with `gpy config get` but not set. `gpy config list` shows every registered key.

**Boolean values accept:** `true`, `false`, `1`, `0`, `yes`, `no`, `on`, `off`

**Directory display modes:**
- `basename` - Show only the current directory name, for example `project`
- `abbreviated` - Show the path with intermediate components shortened to their first character, for example `~/w/p/project`; a hidden directory keeps its leading dot plus one character (`~/.c/fish`)
- `truncated` - Keep the last `ui.directory.truncation_length` path components, prefixed with `ui.directory.truncation_symbol`, for example `…/work/project`
- `full` - Show the whole path with your home directory shown as `~`, for example `~/work/project`

`ui.directory.max_length` applies after the directory mode is rendered, so it can truncate a basename, abbreviated, truncated, or full path if needed.

**Example:**
```bash
gpy config set git.show_upstream false
gpy config set ui.show_icons true
gpy config set ui.directory.display truncated
gpy config set ui.directory.max_length 40
gpy config set language.cache_ttl_hours 48
```

**Output:**
```
Set git.show_upstream = false
```

---

#### `gpy config open`

Open the active config file in your default editor.

This command respects `GPY_CONFIG_PATH` when set. Otherwise it opens the standard config path under `XDG_CONFIG_HOME` or `~/.config/gpy/config.toml`. If the file does not exist yet, GPY creates it first as a short comment-only file that points at the [configuration reference](configuration-reference.md); defaults are not written out.

Editor selection order:
- `$VISUAL`
- `$EDITOR`
- Platform default opener as a fallback: `open -t` on macOS, `xdg-open` on Linux (on WSL, `wslview` when it is installed, then `xdg-open`), `start` on Windows. If the opener cannot be run, the command fails with `no editor found: set $VISUAL or $EDITOR (config file: <path>)`.

When it created the file it prints `Created config: <path>`. After `$VISUAL`/`$EDITOR` (which blocks until the editor exits) nothing more is printed; the platform opener returns immediately, so it prints `Opened <path> in the default editor (<program>)`.

**Example:**
```bash
gpy config open
```

**Output:**
```text
Created config: /home/user/.config/gpy/config.toml
```

(Only when the file did not exist.)

---

#### `gpy config wizard`

Launch an interactive TUI for picking a theme, palette, and enabled segments, with a live preview of the resulting prompt rendered as you navigate.

Keys (inside the wizard, `?` lists the ones that apply to the current view):

| Key | When | Action |
|---|---|---|
| `Tab` | browsing, filtering | next section |
| `Shift+Tab` | browsing, filtering | previous section |
| `Up` / `Left` | browsing, filtering | previous item |
| `Down` / `Right` | browsing, filtering | next item |
| `Space` / `Enter` | browsing, filtering | select/toggle |
| `/` | browsing | filter all sections |
| `?` | browsing | help |
| `s` | browsing | save and exit |
| `q` / `Esc` | browsing | quit |
| `[` | browsing, `directory` highlighted | previous display style |
| `]` | browsing, `directory` highlighted | next display style |
| `[` / `]` | browsing, `language` highlighted | switch icon/text display |
| `v` | browsing, `language` highlighted | show/hide versions |
| `[` / `]` | browsing, `clock` highlighted | switch 12h/24h time |
| `:` | browsing, `clock` highlighted | show/hide seconds |
| `m` | browsing, `duration` highlighted | show/hide milliseconds |
| `b` | browsing, `git` highlighted | show/hide branch |
| `a` | browsing, `git` highlighted | show/hide ahead/behind |
| `t` | browsing, `git` highlighted | show/hide stash |
| `Esc` | filtering | clear and leave filter |
| `Backspace` | filtering | delete last filter character |
| `?` / `Esc` | help open | close help |
| `y` / `Y` | discard prompt | discard changes and quit |

`Space`/`Enter` selects the highlighted theme or palette, or toggles the highlighted segment. The filter narrows all three sections at once; while filtering, any other character is added to the filter. Quitting with unsaved changes asks `Discard changes and quit? [y/N]`; any key other than `y`/`Y` returns to the wizard. The `language` and `directory` options are saved to `config.toml`; the `clock`, `duration` and `git` options are saved to the selected theme's file in `~/.config/gpy/themes/`.

Only theme options whose value you changed are written; an option toggled back to where it started writes nothing. For a builtin or plugin theme, the wizard writes a self-named overlay holding just those keys (for example `starship.toml` with `extends = "starship"` and `[segments.clock]` `show_seconds = true`), so the theme keeps receiving updates. In an existing overlay it sets each changed key, or removes it when it now equals the theme the overlay extends, and drops an `unset` entry for exactly that key; comments are kept. In a full theme file it sets the changed keys in place and suggests `gpy theme rebase`. The file is loaded before it is written; if it would not load, nothing is saved.

Changes are only written on save; quitting without saving leaves the existing configuration and theme files untouched.

After the wizard leaves the full-screen view, `s` prints `Saved theme edits to <path>` (when a theme file was written) and `Saved configuration to <path>` on the normal screen, followed by an `Agent not reloaded` warning only when a running agent rejected the config or did not answer. Quitting without saving prints nothing.

Selecting a user theme that fails to load keeps the wizard open: the selection stays on the previous theme and the Detail panel shows `Cannot select theme "<name>": <reason>` (naming the file) until the next key press. A configured theme that fails to load at startup still aborts the wizard.

**Example:**
```bash
gpy config wizard
```

**Output (illustrative):**
```text
┌ Theme ───────────────────────────────────┐┌ Detail ──────────────────────────┐
│[x] default                               ││default                           │
│( ) starship                              ││                                  │
└──────────────────────────────────────────┘│Currently selected.               │
┌ Palette ─────────────────────────────────┐│Selected value: default           │
│[x] default                               ││Matches the startup config.       │
│( ) nord                                  ││                                  │
│( ) catppuccin-mocha                      ││                                  │
└──────────────────────────────────────────┘│                                  │
┌ Segments ([Space/Enter] to toggle) ──────┐│                                  │
│[x] directory                             ││                                  │
│[x] git                                   ││                                  │
│[x] language                              ││                                  │
│( ) duration                              ││                                  │
│[x] status                                ││                                  │
└──────────────────────────────────────────┘└──────────────────────────────────┘
┌ Preview ─────────────────────────────────────────────────────────────────────┐
│~/code/gpy on main via rust 1.80.0 ✔                                          │
│❯                                                                             │
└──────────────────────────────────────────────────────────────────────────────┘
[?] help  [s] save and exit  [q/Esc] quit
```

The highlighted row is shown in reverse video; the Detail panel describes it. On narrow terminals Detail stacks below the lists. The Preview draws up to three detected languages, as the prompt does.

---

### Diagnostics

#### `gpy doctor`

Run health checks and diagnostics.

Verifies:
- Agent is running and responsive
- Configuration file is valid TOML
- Active theme exists and validates with remediation hints on failure
- Enabled segments are valid
- Each enabled script plugin segment runs once the way the shell runner runs it
  (clean environment, the plugin root as working directory or the current one
  when `pwd` is declared, killed after `timeout_ms`) and reports the run time,
  what it would show and why it would be hidden
- `gpy-agent` runs, and Bash/Fish have an IPC client (`socat`, or an `nc` whose
  help lists `-U`); without one every prompt falls back to `gpy-agent oneshot`.
  The IPC client check runs only when a Bash or Fish integration is installed
  and the agent is not disabled. To tell whether `nc` supports `-U`, doctor
  runs `nc -h` (the first `nc` in `PATH`) when `socat` is absent.

**Example:**
```bash
gpy doctor
```

**Output:**
```
GPY Doctor - System Health Report
=================================

[Agent]
   Checking process... ✓ Running

[Configuration]
   Checking config file... ✓ Valid
   ...

Summary:
✓ All checks passed.
```

With no agent running the first check reads `! Not running` with the hint
``Start the agent with `gpy start` or just use your prompt.``. That is a
warning, not a failure: the command still exits `0`. The same holds for an
agent that is running but too busy to answer (`! Running but too busy to
answer`) and for a stale socket file nothing listens on. An agent that holds
its socket but never answers is a failure (`✗ Not answering on <socket>`,
remediation `gpy restart`), as is a socket path that cannot be resolved.

With `agent.enabled = false` the agent check reads
`Disabled via config (agent.enabled = false)` (the agent is not probed) and
the IPC client check is skipped.

Only a failed check fails the run. Warnings (outdated shell scripts, unknown
config keys, an inactive broken theme override, script trial and plugin
performance findings) are advice and never change the exit code.

**Exit Codes:**
- `0` - No check failed (warnings may be listed above the summary)
- `1` - A check failed

---

#### `gpy debug paths [--format json|kv]`

Dump every path GPY resolves from the environment: the runtime root, the
socket, the shell registry directory, the cache root, the instant-prompt
cache directory, the theme export file, the config path and its candidate
list, and the theme directory. Use it when a shell and the agent seem to
disagree about where something lives.

**Options:**
- `--format json` (default) - one JSON object, keys in resolution order
- `--format kv` - `key=value` lines, the same shape each shell's
  `__gpy_debug_paths` prints; the path-parity tests diff the two

**Example:**
```bash
gpy debug paths --format kv
```

**Output:**
```
runtime_root=/run/user/1000/gpy
socket=/run/user/1000/gpy/gpy.sock
shell_registry_dir=/run/user/1000/gpy/shells
cache_root=/home/user/.cache/gpy
...
```

With `--format json` the list-valued `config_candidates` is a JSON array of
strings (`[]` when there are none); `--format kv` joins it with `|`:

```json
"config_candidates": ["/home/user/.config/gpy/config.toml"],
```

---

#### `gpy debug prompt`

Time the agent round trips a prompt makes for the current directory: a ping
(baseline latency), a git status request and a language detection request,
plus a `Git + Language Total` of the last two. Other segments render in the
shell or through separate requests and are not included. Exits `1` with an
`Error:` line when no agent is running; start it with `gpy start`.

---

### Plugin Commands

Plugin segment files (`entry_type = "file"`) are loaded by the Fish integration only. Script segments (`entry_type = "script"`) run in Bash, Zsh and Fish; see [Plugin Framework](../dev/plugins.md#script-segments).

#### `gpy plugin list`

List discovered plugins, their source, root path, API version, and provided segments.

Useful when:
- checking whether a community plugin was discovered
- confirming which root GPY loaded it from
- seeing discovery diagnostics for malformed plugins

#### `gpy plugin new <id> [--segment <segment>]`

Create a new plugin scaffold in `~/.config/gpy/plugins/<id>/`.

Creates a script (`entry_type = "script"`) plugin that runs in every shell:
- `plugin.toml`
- `segments/<segment>.toml`, the segment manifest (`inputs`, `env`, `ttl_secs`,
  `timeout_ms`)
- `bin/<segment>.sh`, a POSIX `sh` script that prints the segment text

If `--segment` is omitted, GPY uses the plugin id as the segment id.

**Example:**
```bash
gpy plugin new demo-tools --segment demo_status
```

**Output:**
```
Created plugin scaffold
   plugin: demo-tools
   root: /home/user/.config/gpy/plugins/demo-tools
   manifest: /home/user/.config/gpy/plugins/demo-tools/plugin.toml
   segment manifest: /home/user/.config/gpy/plugins/demo-tools/segments/demo_status.toml
   script: /home/user/.config/gpy/plugins/demo-tools/bin/demo_status.sh

Next steps:
   1. Edit the generated files
   2. Keep bin/demo_status.sh and its directories owned by you and not writable by group or others; GPY skips a script segment that fails this check (`gpy plugin validate` says why).
   3. Run `gpy plugin validate /home/user/.config/gpy/plugins/demo-tools`
   4. Run `gpy enable demo_status`
```

#### `gpy plugin install <dir> [--force]`

Copy the plugin in `<dir>` (it holds `plugin.toml`) into `~/.config/gpy/plugins/`.

The source is checked for content and layout only: a source tree with a
symlink, device, FIFO or socket is refused, and the files its manifests name
must exist. The copy gets modes the script trust check accepts (directories
0755, files 0644, or 0755 when the source file is owner-executable; never
group or other writable), then is loaded to confirm it works; a copy that does
not load is removed. Plugin code never runs.

`--force` replaces an installed user plugin with the same id (the old
directory moves to `~/.config/gpy/plugins.bak/<id>.<timestamp>`), and is
needed to shadow a bundled plugin. Backups and staging live outside
`plugins/` because discovery would load them as plugins.

**Output:**
```
Installed plugin demo-tools 0.1.0 into /home/user/.config/gpy/plugins/demo-tools
Enable its segments: gpy enable demo_status
```

#### `gpy plugin remove <id>`

Move the user plugin `<id>` to `~/.config/gpy/plugins.bak/<id>.<timestamp>` and
remove its segments from `ui.enabled_segments`. Plugins that ship with GPY are
refused (disable their segments with `gpy disable`).

#### `gpy plugin validate <id-or-path>`

Validate a plugin manifest and filesystem layout.

Checks:
- `plugin.toml` parses successfully
- the declared plugin API version is supported
- `provided_segments` is present and lists at least one segment
- segment files exist for declared file-based segments
- each file in the plugin's `themes/` directory loads and validates like
  `gpy theme validate <file>`; a plugin overlay (`extends`) must extend a
  builtin theme
- for a script plugin, every `segments/<segment>.toml` manifest parses against
  the published schema and its `command` (and `require_command`) is a file
  inside the plugin root; the output lists each segment's description,
  command, inputs, env, cache scope, `ttl_secs`, `timeout_ms` and triggers
- each script passes `shellcheck -s sh`, or `dash -n` when ShellCheck is not
  installed (a lint failure fails validation; with neither tool installed
  the script is not linted and a warning says so)
- warnings (not failures) for `$VAR` uses the manifest does not provide:
  undeclared `env` names, `GPY_*` inputs missing from `inputs`, and
  undeclared `GPY_OPT_*` settings
- the running GPY meets `compatibility_constraints.min_gpy_version` (a newer
  minimum fails validation, and discovery skips the plugin with a diagnostic)

You can pass either:
- a plugin directory
- a `plugin.toml` path
- a discovered plugin id

---

#### `gpy completions <shell>`

Generate shell completion scripts.

**Supported Shells:**
- `bash`
- `fish`
- `zsh`
- `powershell`
- `elvish`

**Example:**
```bash
# For Fish shell
gpy completions fish > ~/.config/fish/completions/gpy.fish

# For Bash
gpy completions bash > /etc/bash_completion.d/gpy

# For Zsh
gpy completions zsh > ~/.zsh/completions/_gpy
```

---

### Shell Setup

```bash
gpy setup [--shell fish|zsh|bash] [--from <dir>] [--non-interactive]
gpy setup --redetect-icons [--non-interactive]
```

Installs GPY's scripts for one shell and wires them into your rc file. Homebrew
and source builds finish with this command; the curl installer does the same
work itself. Safe to re-run. The shell comes from `--shell`, then `GPY_SHELL`,
then `$SHELL`. `--from` names a directory holding the `fish/`, `zsh/` and
`bash/` trees; without it, `GPY_SHARE_DIR`, then the `share/gpy` beside the
binary, then the source checkout it was built in are tried. See
[`gpy setup`](../INSTALL.md#gpy-setup) for the full list of steps.

The Bash block loads GPY only in interactive shells. Setup rewrites an
unedited Bash block or login loader from an earlier release to that form and
prints `Updated the GPY block in <file>`; a block whose text differs is left
as it is, and setup says so.

A theme file under `~/.config/gpy/themes/` named after a builtin theme replaces
that builtin entirely. Setup moves one that is byte-identical to a version of
the builtin released up to 0.7.1 to `<name>.toml.bak` and reports it, so the
builtin applies again; an edited copy or a symlink is kept and listed. Set
`ui.retire_unmodified_themes` to `false` to keep every copy.

The last step picks `ui.show_icons`: it detects whether a Nerd Font is
installed and, on a terminal, shows a sample prompt line and asks whether the
icons render (detection only sets the default answer). An existing config is
left alone. `--redetect-icons` runs only this step and overwrites
`ui.show_icons` in place, keeping the rest of the file; use it after
installing or removing a Nerd Font. `GPY_NERD_FONT` answers the question
without a prompt (see the environment table), as does `--non-interactive`,
which takes the detected default.

### Uninstall

```bash
gpy uninstall [--yes] [--purge-config]
```

Removes what `gpy setup` and the installers create: the shell scripts and
completions, the GPY block in your rc files (restored byte for byte), the fish
prompt link (restoring your previous prompt), the cache and runtime files, and
the running agent. `~/.config/gpy` is kept unless you pass `--purge-config`.
The binaries are deleted only when this `gpy` lives in `~/.local/bin`, where the
installers put it; otherwise remove them with the tool that installed them
(`brew uninstall gpy` for Homebrew). It asks `Uninstall GPY? [y/N]`; with no
terminal to answer, it removes nothing and exits `1` unless you pass `--yes`.
See [Uninstallation](../INSTALL.md#uninstallation).

### Shell Completion

Tab-completion for the `gpy` CLI is installed automatically by the standard installers: the one-line installer sets up completions for your detected shell (fish, bash, or zsh), and `install.sh` sets up fish. Completions are regenerated with every install or upgrade, so they stay in sync with your binary.

**Structural completion** (all shells):
- `gpy <TAB>` completes subcommands and flags

**Dynamic completion** (live from what's installed):
- `gpy theme use <TAB>`, `gpy theme validate <TAB>`, `gpy theme diff <TAB>`, `gpy theme reset <TAB>`, `gpy theme migrate <TAB>` and `gpy theme rebase <TAB>` — installed theme names
- `gpy palette use <TAB>` and `gpy palette validate <TAB>` — installed palette names
- `gpy enable <TAB>` and `gpy disable <TAB>` — available segment names

The `gpy completions <shell>` command (documented above) is the underlying generator. The standard installers run it for you. Completions require the `gpy` binary to be on your PATH.

---

## Common Workflows

### Minimal Prompt for Screenshots

```bash
gpy disable git
gpy disable language
gpy disable clock
gpy theme use text
```

### Performance Mode

Disable expensive operations for faster prompts:

```bash
gpy disable git
gpy disable language
```

### Full-Featured Prompt

```bash
gpy theme use default
gpy enable git
gpy enable language
gpy config set language.show_versions true
```

### Theme Customization

```bash
# Create a custom theme
gpy theme new mytheme

# Edit the theme file
$EDITOR ~/.config/gpy/themes/mytheme.toml

# Activate your theme
gpy theme use mytheme
```

### Troubleshooting

```bash
# Check everything is working
gpy doctor

# View detailed configuration
gpy config show

# Check agent status
gpy status

# Restart if needed
gpy restart
```

---

## Configuration File

The main configuration file is located at `~/.config/gpy/config.toml`.

You can edit it directly or use `gpy config set` commands to update values programmatically.

**Example config.toml:**

```toml
[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[git]
enabled = true
show_upstream = true
timeout_seconds = 10

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24

[ui]
show_icons = true
theme = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]

[ui.directory]
display = "basename"
truncation_length = 3
truncation_symbol = ""
max_length = 80
```

---

## The `gpy-agent` binary

`gpy-agent` is the daemon. The shell integrations call it directly, and
`gpy start/stop/restart/status` delegate to it, so these are supported
commands.

| Command | What it does |
|---|---|
| `gpy-agent start [--socket <path>] [--quiet]` | Start the daemon in the background (idempotent; replaces an agent whose version or protocol differs, leaves a busy one alone) and print one outcome line with its PID. `--socket` overrides the socket path, mainly for tests; `--quiet` prints nothing on success. |
| `gpy-agent stop [--socket <path>] [--quiet]` / `gpy-agent status [--socket <path>]` | Same behaviour and exit codes as the `gpy` equivalents above. |
| `gpy-agent restart [--socket <path>] [--quiet]` | Stop the agent (a failed stop is a `warn:` line, not an abort), then start it; prints the stop line and the start line and exits with `start`'s code. |
| `gpy-agent oneshot <git\|lang\|directory\|duration\|character\|exit-status> [--cwd <dir>] [--format json\|ansi\|bash-prompt\|zsh-prompt]` | Render one segment once, with no daemon. The shells use it as the fallback when the agent is down; it is also the quickest way to see what a segment produces for a directory. A failure prints one `Error:` line on stderr and exits `1`. The old name `status` for `exit-status` is deprecated and prints a warning on stderr. |
| `gpy-agent theme export --shell fish\|zsh\|bash` | Print the active theme as shell variable assignments. Each shell sources this on init (cached under `$XDG_CACHE_HOME/gpy/theme-export.<shell>`). The old `--format` flag for the shell is deprecated and prints a warning on stderr. |
| `gpy-agent config reload [--socket <path>]` | Ask the running agent to reload `config.toml` from disk. Exits `0` when the agent applied it, or when no agent is running (the config applies at the next start). Exits `1` with an `Error:` line when the agent rejects the config, carrying its reason (the agent keeps its previous configuration), or when a running agent does not answer. |

---

## Environment Variables

GPY reads the following environment variables:

| Variable | Effect |
|---|---|
| `GPY_CONFIG_PATH` | Explicit path to `config.toml`; checked before the XDG and `$HOME` locations. An empty value is ignored. |
| `XDG_CONFIG_HOME` | Config root: `$XDG_CONFIG_HOME/gpy` (default `~/.config/gpy`), also where user themes and palettes live. |
| `XDG_CACHE_HOME` | Cache root: `$XDG_CACHE_HOME/gpy` (default `~/.cache/gpy`): theme exports, instant-prompt cache, the agent log. Also the runtime root when `XDG_RUNTIME_DIR` is unset. |
| `XDG_RUNTIME_DIR` | Runtime root: `$XDG_RUNTIME_DIR/gpy`: the socket and the shell registry. |
| `GPY_AGENT_SOCKET_PATH` | Override the socket path alone (the runtime root is unchanged). Every command and every shell must see the same value. Every socket, default or override, also gets a `<socket>.lock` start lock next to it. Its directory must be a real directory you own (see [`gpy start`](#gpy-start)). |
| `HOME` | Fallback for the config and cache roots when the XDG variables are unset. |
| `GPY_NERD_FONT` | Tells `gpy setup` what to assume about icons instead of asking: `1`/`true`/`yes`/`on`/`nerd` (Nerd Font present), `0`/`false`/`no`/`off`/`none`/`ascii` (absent), `unknown`; anything else falls through to detection. Setting it skips the prompt. Under WSL detection always reports `unknown` (the terminal's fonts are on the Windows side), so set this to answer for it. |
| `VISUAL`, `EDITOR` | The editor `gpy config open` launches, in that order, before the platform fallback. |
| `GPY_DEBUG_LOG` | Path of the agent's trace log; the agent writes its debug lines there only when this is set. (`GPY_DEBUG=1` turns on the *shell* integrations' own debug output.) See [Troubleshooting](troubleshooting.md). |
| `GPY_IPC_TIMEOUT_MS` | Shell integrations only (Fish, Zsh, Bash alike): how long a prompt render waits for the agent's reply, in milliseconds. Default `150`. A value that is not a non-negative integer falls back to the default. Export it in `config.fish`, `.zshrc` or `.bashrc` before the GPY integration is sourced. An agent that is reached but replies later is not recomputed with a `gpy-agent oneshot` fork: that segment is omitted for the render. |
| `GPY_IPC_COLD_TIMEOUT_MS` | Shell integrations only: the same wait when the prompt has nothing to show for a segment yet (a git instant cache that is cold, a directory not yet rendered in this shell), so a loaded machine still draws it. Default `500`, never shorter than `GPY_IPC_TIMEOUT_MS`; set and validated like `GPY_IPC_TIMEOUT_MS`. |
| `GPY_GIT_INSTANT_CACHE_TTL_SECONDS` | Shell integrations only: seconds after which the git instant-cache entry counts as stale. A stale entry is still shown at once and triggers a throttled background refresh. Default `5`; set and validated like `GPY_IPC_TIMEOUT_MS`. |
| `GPY_LANGUAGE_CACHE_TTL_SECONDS` | Shell integrations only: the same for the language instant-cache entry. Default `30`. |

`gpy debug paths` prints what the path variables resolve to on the current machine. An empty `HOME` counts as unset in every shell and in the agent: the passwd database's home directory is used instead.

### Fish-only switches

These exist in the Fish integration only; Bash and Zsh have no counterpart. `GPY_SHOW_STATUS` is read on every render.

| Switch | Effect |
|---|---|
| `GPY_SHOW_STATUS` | `0` hides the standalone `✔`/`✖` exit-status indicator printed before the prompt symbol. Default `1`. The indicator is never shown while the prompt symbol itself is rendered by the agent (it is already coloured by the exit status), so this matters mainly when the agent is unavailable. |

### Shell commands

Functions the integration defines in Fish, Bash and Zsh (not executables).

| Command | Effect |
|---|---|
| `gpy-prompt-reload` | Re-reads `config.toml` into the running shell by running `gpy-agent theme export` (never the cache file, which a stopped agent leaves stale), so it works whether the agent is running, disabled or stopped. Silent on success; on failure it prints `gpy: theme reload failed; run gpy doctor` and returns 1. An invalid `config.toml` applies the built-in defaults, shows the agent's `Warning: [Config loading] …` line and returns 0. In Fish, `prompt-reload` is its deprecated former name. |

Fish also has `gpy-prompt-debug` (`validate`, `benchmark`, `cache`, `vars`; `prompt-debug` is its deprecated former name). In every shell, `gpy doctor`, `gpy debug prompt` and `gpy debug paths` are the debug commands.

---

## Exit Codes

| Command | 0 | 1 | 2 | 3 | 4 | 6 |
|---|---|---|---|---|---|---|
| `gpy status`, `gpy-agent status` | the agent answered, or it is alive but shedding load (busy) | – | usage error | not running: no socket, or the socket refuses connections (stale file) | the socket accepts but the agent never replies (wedged), or the status could not be determined (an `Error:` line names it) | – |
| `gpy start`, `gpy-agent start` | agent running afterwards: started, replaced an older agent, already running (including busy), or a newer agent left in place | error, including "could not verify whether an agent is running" | usage error | – | – | disabled by `agent.enabled = false` |
| `gpy stop`, `gpy-agent stop` | agent stopped afterwards, including "was not running" and "removed a stale socket" | the agent is still answering or did not answer the shutdown | usage error | – | – | – |
| `gpy restart`, `gpy-agent restart` | as `start` | as `start` | usage error | – | – | as `start` |
| `gpy-agent config reload` | applied, or no agent running (unchanged) | rejected or not answered | usage error | – | – | – |
| `gpy doctor` | no failed check (warnings allowed) | a check failed | usage error | – | – | – |
| `gpy uninstall` | uninstalled | not confirmed (no terminal and no `--yes`, or the answer was not `y`), or a path or rc file could not be handled | usage error | – | – | – |
| `gpy-agent oneshot`, `gpy-agent theme export`, every other `gpy` command | success | failure: one line starting with `Error:` on stderr | usage error | – | – | – |

`stop` never exits `6`: a disabled agent that is not running is already in the
requested state, and one that is still running must be stoppable. `2` is the
argument parser's usage error: an unknown subcommand, a missing argument, or a
value outside the allowed set (for example `gpy config show nosuch`, which
lists the possible values). Every one of these commands lists its codes in
`--help`.

Output piped into a reader that stops early (`gpy segments | head -1`,
`gpy completions fish | head -c 1`) ends quietly with `0`; it is never
reported as an error.

---

## See Also

- [Advanced Configuration](advanced-configuration.md) - Detailed customization options
- [Installation + Troubleshooting](../INSTALL.md#troubleshooting) - Common issues and solutions
- [Architecture](../dev/architecture.md) - System design and internals

---

## Getting Help

For issues and questions:

- **GitHub Issues**: https://github.com/jpease/gpy/issues
- **Discussions**: https://github.com/jpease/gpy/discussions
- **Built-in help**: `gpy <command> --help`
