# GPY Command-Line Interface Reference

Complete reference for the `gpy` command-line tool.

## Overview

`gpy` is the user-facing command-line interface for managing the GPY prompt for Bash, Fish and Zsh. It provides a simple, intuitive way to control the agent, manage themes, configure segments, and customize your prompt experience.

A second binary, `gpy-agent`, is the daemon itself. It also carries the subcommands the shell integrations and installers call (`init`, `oneshot`, `theme export`, `config get|list|reload`); they are listed under [The `gpy-agent` binary](#the-gpy-agent-binary) below.

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
gpy theme new <name> [--from <base>]  # Create new theme, optionally cloning <base>
gpy theme save [name]        # Save the active theme's contents to a theme file
gpy theme validate [target]  # Validate active theme, name, or file path

# Palette management
gpy palette list                 # List available palettes
gpy palette use <name>           # Switch to a palette
gpy palette show                 # Show current palette
gpy palette validate [target]    # Validate active palette, name, or file path
gpy palette import <file>        # Import a base16/base24 scheme as a palette

# Plugin management
gpy plugin list                              # List discovered plugins
gpy plugin new <id> [--segment <segment>]    # Create plugin scaffold
gpy plugin validate <path-or-id>             # Validate plugin files and manifest

# Segment management
gpy enable <segment>         # Enable a segment
gpy disable <segment>        # Disable a segment
gpy segments                 # List all segments

# Language configuration
gpy lang versions on|off     # Toggle version display

# Configuration
gpy config show [section]    # Show configuration
gpy config get <key>         # Get a config value
gpy config set <key> <value> # Set a config value
gpy config wizard            # Launch the interactive configuration wizard

# Diagnostics
gpy doctor                   # Run health checks
gpy completions <shell>      # Generate shell completions
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

**Output** (on stderr; stdout stays empty so the command is safe in pipelines):
```
Starting GPY Agent in background...
```

The command returns once the daemon answers on its socket; `gpy status`
then reports `Running and Responding`. Starting is idempotent: with a
matching agent already running it exits `0` without forking a second one,
and with an older agent running it evicts it first (`Detected version
mismatch`) so an upgrade never leaves a stale daemon serving new shells.
A newer running agent is left alone. If the new daemon exits or does not
answer within 5 seconds, `gpy start` (and `gpy-agent start`) exits `1`
with an `Error:` line. The daemon runs in `/`, so it never keeps the
directory you started it from (or its volume) busy; a relative `--socket`,
`GPY_AGENT_SOCKET_PATH`, `GPY_CONFIG_PATH` or `GPY_DEBUG_LOG` still
resolves against that directory.

---

#### `gpy stop`

Stop the running background agent.

Sends a graceful shutdown signal to the agent.

**Example:**
```bash
gpy stop
```

**Output:**
```
Shutdown command sent successfully
Agent stopped successfully
```

Stopping is idempotent: with no agent running it prints
`Agent is not running (socket not found)` and still exits `0`.

A socket file with nothing listening is a leftover, not a running agent: `stop`
removes it, prints `Agent is not running (stale socket removed)` and exits `0`.
If an agent is there but cannot be stopped (it does not answer the shutdown
request, or still answers after the shutdown wait), `stop` prints one `Error:`
line on stderr and exits `1`.

---

#### `gpy restart`

Restart the background agent.

Equivalent to running `gpy stop` followed by `gpy start`, reporting each
step as it completes.

**Example:**
```bash
gpy restart
```

**Output:**
```
Restarting GPY...
✅ Shutdown command sent
✅ GPY stopped
✅ GPY started
```

---

#### `gpy status`

Display comprehensive agent status and diagnostics.

Shows:
- Socket path and configuration file location
- Whether the agent answered (`Running and Responding`), is absent, or
  failed to answer
- Agent version and protocol version
- Watched repositories, registered clients and cache entries

**Example:**
```bash
gpy status
```

**Output:**
```
GPY Agent Status
================
Socket Path: /run/user/1000/gpy/gpy.sock
Config: /home/user/.config/gpy/config.toml

Status: Running and Responding
Agent Version: 0.1.0
Protocol Version: 1
Watched Repos: 1
Registered Clients: 1
Cache Entries: 2
```

**Exit Codes:**
- `0` - The agent answered (`Status: Running and Responding`)
- `1` - No agent is running (`Status: Not Running (socket not found)`) or the
  socket exists but nothing answered (`Status: Connection failed - ...`)

The exit code is what scripts and `gpy doctor` rely on; the report is
printed either way. `gpy-agent status` follows the same contract.

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

  default *
  text
```

The `*` marker indicates the currently active theme.

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
  carries recommendations prints a `💡 Re-run with --force to apply
  recommended ...` hint and leaves your layout alone.

**Output:**
```
✅ Switched to 'text' theme
```

That is the whole output when the running agent acknowledged the reload.
When it did not, one more line follows:

```
⚠️  Agent not reloaded; the new config applies the next time it starts.
```

Every command that edits `config.toml` (`theme use`, `palette use`,
`enable`, `disable`, `lang versions`, `config set`) ends the same way: silent
on a confirmed reload, that one notice otherwise. Switching *themes* currently
prints the notice even with a live agent on macOS, because the switch re-arms
its theme-file watcher for longer than the CLI waits (#661); the reload still
happens a few seconds later.

---

#### `gpy theme import <path>`

Import a `starship.toml` into a matching GPY palette and prompt theme pair.
The migration guide is in [Migrating from Starship](migrating-from-starship.md).

**Arguments:**
- `<path>` - Path to the source `starship.toml`

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

#### `gpy theme new <name> [--from <base>]`

Create a new theme based on the default theme template, or by cloning an existing theme.

Destination names must satisfy the safe configuration name invariant: non-empty,
non-whitespace, containing no path separators (`/`, `\`), `..`, or control characters.
An invalid destination name is rejected before creating files or directories.

Creates a new theme file in `~/.config/gpy/themes/<name>.toml`. Without `--from`, it copies the default theme. With `--from <base>`, it copies the full contents of `<base>` (a builtin, user, or plugin theme name) instead, so you can start customizing from a theme you already like rather than the blank default. You can then edit this file to customize colors, icons, and other visual elements.

**Arguments:**
- `<name>` - New theme name (non-empty safe config name)

**Options:**
- `--from <base>` - Clone an existing theme's full contents instead of the default template

**Example:**
```bash
gpy theme new mytheme

# Start from the starship preset instead of the default template
gpy theme new mytheme --from starship
```

**Output:**
```
✅ Created new theme: /home/user/.config/gpy/themes/mytheme.toml
📝 Edit: /home/user/.config/gpy/themes/mytheme.toml
🎨 Activate with: gpy theme use mytheme
```

---

#### `gpy theme save [name]`

Save the currently active theme's contents to a theme file.

With `<name>`, saves to `~/.config/gpy/themes/<name>.toml`: created without prompting if no theme by that name exists yet, or overwritten after a `[y/N]` confirmation if it does. Without `<name>`, saves over the currently active theme's own name — this always prompts `[y/N]` first, since it always means overwriting "the theme in use", even if that theme is still a builtin with no file on disk yet.

Declining the prompt (anything other than `y`/`yes`) aborts without writing.

**Arguments:**
- `[name]` - Theme name to save as (defaults to the currently active theme)

**Example:**
```bash
# Materialize the active theme (e.g. a builtin) as an editable user theme
gpy theme save

# Save the active theme's contents under a new name
gpy theme save mytheme
```

**Output:**
```
✅ Saved theme: /home/user/.config/gpy/themes/mytheme.toml
📝 Edit: /home/user/.config/gpy/themes/mytheme.toml
🎨 Activate with: gpy theme use mytheme
```

---

#### `gpy theme validate [target]`

Validate a theme and print actionable diagnostics.

Validates theme schema fields, icon definitions, and segment format templates
across all eight supported agent-rendered segments (`git`, `language`, `directory`,
`duration`, `character`, `clock`, `hostname`, `username`) against the active palette,
verifying that all color tokens resolve to valid ANSI colors or defined palette roles.

If `target` is omitted, validates the currently active theme from config.
If `target` is provided, it can be either:
- a theme name (for discovered themes), or
- a direct path to a theme `.toml` file.
- `[target]` - Optional theme name or file path

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
✅ Theme validation passed
   target: default
   source: active config theme
```

**Failure output (example):**
```
❌ theme validate failed
   reason: Configuration error: Invalid color 'not-a-color' for field ui.prompt_color in theme /home/user/.config/gpy/themes/mytheme.toml
   remediation:
   - Use named colors, ANSI 0-255, or #RRGGBB values.
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
✅ Switched to 'nord' palette
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

#### `gpy palette validate [target]`

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
✅ Palette validation passed
   target: nord
   source: named palette
```

**Failure output (example):**
```
❌ palette validate failed
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
✅ Imported palette 'gruvbox-hard' → /home/user/.config/gpy/palettes/gruvbox-hard.toml
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
- `status` - Previous command exit status pill (✔/✖)
- `username` - Current user name; shown only as root or under sudo (opt-in)
- `hostname` - Machine hostname; shown only over SSH unless `segments.hostname.show_always` is set (opt-in). Its icon (`segments.hostname.icon`, Starship's `ssh_symbol`) shows only in SSH sessions

**Example:**
```bash
gpy enable git
gpy enable language
gpy enable clock
```

**Output:**
```
✅ Enabled git segment
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
✅ Disabled clock segment
```

---

#### `gpy segments`

List all available segments and their current status.

Shows which segments the prompt will actually render (✓) and which it won't ( ). `git` and `language` show ✓ only when they are listed in `ui.enabled_segments` **and** their feature flag is on; every other segment shows ✓ when it is listed. `gpy config wizard` starts its segment checkboxes from the same rule.

**Example:**
```bash
gpy segments
```

**Output:**
```
Prompt Segments:
================

[✓] clock
[✓] duration
[✓] language
[✓] directory
[✓] git
[ ] status
[ ] username
[ ] hostname
```

---

### Language Configuration

#### `gpy lang versions <on|off>`

Toggle language version display in the prompt.

When enabled, the language segment shows version numbers (e.g., `Rust 1.90`). When disabled, only the language name or icon is shown.

**Arguments:**
- `on` - Enable version display
- `off` - Disable version display

**Example:**
```bash
gpy lang versions on
```

**Output:**
```
✅ Language versions enabled
```

---

### Configuration Management

#### `gpy config show [section]`

Display configuration values.

Without a section argument, shows all configuration. With a section argument, shows only that section. Every field is printed, including the `[git.icons]` and `[language.icons]` tables; the output is valid TOML (section headings are comments).

**Valid Sections:**
- `agent` - Agent process settings
- `git` - Git detection settings
- `language` - Language detection settings
- `ui` - User interface settings

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
- `ui.directory.display` - Directory segment display mode: `basename`, `abbreviated`, `truncated`, or `full`
- `ui.directory.truncation_length` - Path components kept when `display = "truncated"` (default `3`)
- `ui.directory.truncation_symbol` - Prefix shown before a truncated path, e.g. `…/` (default empty)
- `ui.directory.max_length` - Maximum displayed length for the rendered directory string
- `language.display` - Language segment display: `icon` or `text`
- `language.filter` - Languages to show: `all`, `primary`, or top N (e.g. `3`)
- `language.detection_mode` - Detection strategy: `content`, `markers`, or `hybrid`
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
✅ Set git.show_upstream = false
```

---

#### `gpy config open`

Open the active config file in your default editor.

This command respects `GPY_CONFIG_PATH` when set. Otherwise it opens the standard config path under `XDG_CONFIG_HOME` or `~/.config/gpy/config.toml`. If the file does not exist yet, GPY creates it first as a short comment-only file that points at the [configuration reference](configuration-reference.md); defaults are not written out.

Editor selection order:
- `$VISUAL`
- `$EDITOR`
- Platform default opener as a fallback: `open -t` on macOS, `xdg-open` on Linux (on WSL, `wslview` when it is installed, then `xdg-open`), `start` on Windows

**Example:**
```bash
gpy config open
```

**Output:**
```text
Opened config: /home/user/.config/gpy/config.toml
```

---

#### `gpy config wizard`

Launch an interactive TUI for picking a theme, palette, and enabled segments, with a live preview of the resulting prompt rendered as you navigate.

Keys: `Tab` moves to the next section, `Space`/`Enter` toggles or selects the highlighted item, `s` saves and exits, `q` quits (prompting to discard changes if any are pending).

Changes are only written to the config file on save; quitting without saving leaves the existing configuration untouched.

After the wizard leaves the full-screen view, `s` prints `✅ Saved configuration to <path>` on the normal screen, followed by the `⚠️  Agent not reloaded` notice when no running agent confirmed the reload. Quitting without saving prints nothing.

Selecting a user theme that fails to load keeps the wizard open: the selection stays on the previous theme and the Detail panel shows `Cannot select theme "<name>": <reason>` (naming the file) until the next key press. A configured theme that fails to load at startup still aborts the wizard.

**Example:**
```bash
gpy config wizard
```

**Output (illustrative):**
```text
┌ Theme ─────────────────────────────────────────────┐
│ > default    text    starship                       │
├ Palette ───────────────────────────────────────────┤
│   default    starship    nord    catppuccin-mocha   │
├ Segments ──────────────────────────────────────────┤
│ [x] git   [x] language   [ ] duration   [x] status   │
├ Preview ───────────────────────────────────────────┤
│ ~/code/gpy (main) rust:1.80.0                        │
└──────────────────────────────────────────────────────┘
[tab] next section  [space/enter] toggle/select  [s] save & exit  [q] quit
```

---

### Diagnostics

#### `gpy doctor`

Run comprehensive health checks and diagnostics.

Verifies:
- Agent is running and responsive
- Configuration file is valid TOML
- Active theme exists and validates with remediation hints on failure
- Enabled segments are valid
- Required binaries are in PATH

**Example:**
```bash
gpy doctor
```

**Output:**
```
GPY Doctor - System Health Report

[Agent]
  Checking process... ✅ Running

[Configuration]
  Checking config file... ✅ Valid
  ...

Summary:
✅ Your GPY environment is healthy and ready to go!
```

With no agent running the first check reads `⚠️  Not running` with the hint
``Start the agent with `gpy start` or just use your prompt.``, and the
command exits `1`.

With `agent.enabled = false` the agent check reads
`ℹ️  Disabled via config (agent.enabled = false)` and is not counted as a
failure (the agent is not probed), so the command can still exit `0`.

**Exit Codes:**
- `0` - All checks passed
- `1` - One or more checks failed (including the agent not running, unless it is disabled via `agent.enabled = false`)

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

---

#### `gpy debug prompt`

Time the agent round trips a prompt makes for the current directory: a ping
(baseline latency), a git status request and a language detection request,
plus a `Git + Language Total` of the last two. Other segments render in the
shell or through separate requests and are not included. Exits `1` with an
`Error:` line when no agent is running; start it with `gpy start`.

---

### Plugin Commands

#### `gpy plugin list`

List discovered plugins, their source, root path, API version, and provided segments.

Useful when:
- checking whether a community plugin was discovered
- confirming which root GPY loaded it from
- seeing discovery diagnostics for malformed plugins

#### `gpy plugin new <id> [--segment <segment>]`

Create a new plugin scaffold in `~/.config/gpy/plugins/<id>/`.

Creates:
- `plugin.toml`
- `segments/<segment>.fish`

If `--segment` is omitted, GPY uses the plugin id as the segment id.

**Example:**
```bash
gpy plugin new demo-tools --segment demo_status
```

#### `gpy plugin validate <path-or-id>`

Validate a plugin manifest and filesystem layout.

Checks:
- `plugin.toml` parses successfully
- the declared plugin API version is supported
- `provided_segments` is present and lists at least one segment
- segment files exist for declared file-based segments

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

### Shell Completion

Tab-completion for the `gpy` CLI is installed automatically by the standard installers — the one-line installer sets up completions for your detected shell (fish, bash, or zsh), and `install.sh` sets up fish. Completions are regenerated with every install or upgrade, ensuring they stay in sync with your installed binary.

**Structural completion** (all shells):
- `gpy <TAB>` completes subcommands and flags

**Dynamic completion** (live from what's installed):
- `gpy theme use <TAB>` and `gpy theme validate <TAB>` — installed theme names
- `gpy palette use <TAB>` and `gpy palette validate <TAB>` — installed palette names
- `gpy enable <TAB>` and `gpy disable <TAB>` — available segment names

The `gpy completions <shell>` command (documented above) is the underlying generator. The standard installers run this automatically; manual redirection is no longer needed for typical installations. Completions require the `gpy` CLI binary to be on your PATH.

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
gpy lang versions on
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

`gpy-agent` is the daemon. The shell integrations and installers call it
directly, and `gpy start/stop/restart/status` delegate to the same code, so
these are supported commands rather than internals:

| Command | What it does |
|---|---|
| `gpy-agent start [--socket <path>]` | Start the daemon in the background (idempotent; evicts an older version). `--socket` overrides the socket path, mainly for tests. |
| `gpy-agent stop` / `gpy-agent status` | Same behaviour and exit codes as the `gpy` equivalents above. Restarting is only available as `gpy restart`. |
| `gpy-agent init [--non-interactive] [--force]` | First-install bootstrap: detects whether a Nerd Font is available and writes `config.toml` with a matching `show_icons`. No-op when a config file exists (unless `--force`). Set `GPY_NERD_FONT` to skip the terminal prompt (see the environment table). The installers run this. |
| `gpy-agent oneshot <git\|lang\|directory\|duration\|character\|hostname\|username> [--cwd <dir>] [--format json\|ansi\|fish\|fish-source\|zsh\|zsh-source]` | Render one segment once, with no daemon. The shells use it as the fallback when the agent is down; it is also the quickest way to see what a segment produces for a directory. |
| `gpy-agent theme export --format fish\|zsh\|bash` | Print the active theme as shell variable assignments. Each shell sources this on init (cached under `$XDG_CACHE_HOME/gpy/theme-export.<shell>`). |
| `gpy-agent theme validate [target]` / `gpy-agent theme import <path>` / `gpy-agent palette ...` | Same as the `gpy theme` and `gpy palette` equivalents. |
| `gpy-agent config get <key>` / `config list` / `config reload` | Read one value, list every key, or ask the running agent to reload `config.toml` from disk. `config reload` prints `Config reload request sent (agent may not have acknowledged).` when no acknowledgement arrived in time. |
| `gpy-agent doctor` | Same report as `gpy doctor`. |

---

## Environment Variables

GPY reads the following environment variables:

| Variable | Effect |
|---|---|
| `GPY_CONFIG_PATH` | Explicit path to `config.toml`; checked before the XDG and `$HOME` locations. An empty value is ignored. |
| `XDG_CONFIG_HOME` | Config root: `$XDG_CONFIG_HOME/gpy` (default `~/.config/gpy`), also where user themes and palettes live. |
| `XDG_CACHE_HOME` | Cache root: `$XDG_CACHE_HOME/gpy` (default `~/.cache/gpy`): theme exports, instant-prompt cache, the agent log. Also the runtime root when `XDG_RUNTIME_DIR` is unset. |
| `XDG_RUNTIME_DIR` | Runtime root: `$XDG_RUNTIME_DIR/gpy`: the socket, `agent.version`, the shell registry. |
| `GPY_AGENT_SOCKET_PATH` | Override the socket path alone (the runtime root is unchanged). Every command and every shell must see the same value. The agent's version marker then lives at `<socket>.version` instead of `agent.version`. Every socket, default or override, also gets a `<socket>.lock` start lock next to it. |
| `HOME` | Fallback for the config and cache roots when the XDG variables are unset. |
| `GPY_NERD_FONT` | Tells `gpy-agent init` what to assume instead of asking: `1`/`true`/`yes`/`on`/`nerd` (Nerd Font present), `0`/`false`/`no`/`off`/`none`/`ascii` (absent), `unknown`; anything else falls through to detection. Setting it makes `init` non-interactive. Under WSL detection always reports `unknown` (the terminal's fonts are on the Windows side), so set this to answer for it. |
| `VISUAL`, `EDITOR` | The editor `gpy config open` launches, in that order, before the platform fallback. |
| `GPY_DEBUG_LOG` | Path of the agent's trace log; the agent writes its debug lines there only when this is set. (`GPY_DEBUG=1` turns on the *shell* integrations' own debug output.) See [Troubleshooting](troubleshooting.md). |
| `GPY_IPC_TIMEOUT_MS` | Shell integrations only (Fish, Zsh, Bash alike): how long a prompt render waits for the agent's reply, in milliseconds. Default `150`. A value that is not a non-negative integer falls back to the default. Export it in `config.fish`, `.zshrc` or `.bashrc` before the GPY integration is sourced. An agent that is reached but replies later is not recomputed with a `gpy-agent oneshot` fork: that segment is omitted for the render. |
| `GPY_GIT_INSTANT_CACHE_TTL_SECONDS` | Shell integrations only: seconds after which the git instant-cache entry counts as stale. A stale entry is still shown at once and triggers a throttled background refresh. Default `5`; set and validated like `GPY_IPC_TIMEOUT_MS`. |
| `GPY_LANGUAGE_CACHE_TTL_SECONDS` | Shell integrations only: the same for the language instant-cache entry. Default `30`. |

`gpy debug paths` prints what the path variables resolve to on the current machine. An empty `HOME` counts as unset in every shell and in the agent: the passwd database's home directory is used instead.

### Fish-only switches

These exist in the Fish integration only; Bash and Zsh have no counterpart. `GPY_SHOW_STATUS` is read on every render; the segment switches are read once, when the integration loads.

| Switch | Effect |
|---|---|
| `GPY_SHOW_STATUS` | `0` hides the standalone `✔`/`✖` exit-status indicator printed before the prompt symbol. Default `1`. The indicator is never shown while the prompt symbol itself is rendered by the agent (it is already coloured by the exit status), so this matters mainly when the agent is unavailable. |
| `GPY_SHOW_LANGUAGES` | `0` replaces the enabled-segment list with `clock duration directory git` at startup, overriding `enabled_segments` from `config.toml`. Prefer editing `enabled_segments`. |
| `GPY_MINIMAL_SEGMENTS` | Any value (even empty) renders only the `directory` segment. A debugging and test hook; it takes precedence over `GPY_SHOW_LANGUAGES`. |
| `GPY_TEST_SEGMENTS` | A space-separated segment list that replaces `enabled_segments`, for tests (`GPY_MINIMAL_SEGMENTS` wins when both are set). |
| `prompt-reload` | A function, not a variable: reloads the theme and config into the running shell, mainly for debugging or when the agent is disabled. |

---

## Exit Codes

- `0` - Success
- `1` - The command failed; stderr carries one line starting with `Error:`
  that names the problem (an unknown theme, segment, config key, section or
  value, an agent that could not be reached, ...). `gpy status` also exits
  `1` when no agent is running, with the report on stdout and no `Error:` line.
- `2` - Usage error from the argument parser: an unknown subcommand, a missing
  argument, or a value outside the allowed set (for example
  `gpy lang versions maybe`, which lists the possible values).

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
