# GPY CLI Specification

> **Archived.** This document describes specifications that may be superseded or historical. Refer to current documentation for active CLI behavior.
>
> The `gpy_config_validate` Fish function this spec describes `gpy doctor` as wrapping was retired (#662); `gpy doctor` no longer shells out to it. `gpy_setup` was retired alongside it. Current setup/validation is `gpy-agent init` / `gpy config wizard` and `gpy doctor`.

## Overview

Introduce a user-facing `gpy` command-line tool that complements the existing `gpy-agent` daemon. The new binary provides high-level management commands (themes, agent lifecycle, diagnostics) while delegating background work to the agent. This improves usability by consolidating day-to-day operations under a single ergonomic command.

## Goals

- **One-stop CLI**: expose the most common workflows (theme selection, agent status, restart) via `gpy <command>`.
- **Theme management**: list available themes, activate them, scaffold new theme files, and proxy theme export operations.
- **Agent lifecycle shortcuts**: surface `status`, `start`, `stop`, `restart`, and `doctor` commands without requiring direct `gpy-agent` usage.
- **Configuration helpers**: provide read/write tooling for `config.toml` fields that users frequently edit (`ui.theme`, enabled segments, etc.).
- **Reuse existing logic**: leverage `gpy-agent` library modules (config loader, cache policy, theme validation) so behaviour remains consistent.

## Non-Goals

- Replacing the agent or IPC protocol. The CLI is a thin orchestration layer.
- Managing Fish shell startup hooks or prompt rendering logic.
- Providing TUI/interactive editors beyond simple prompts (future follow-up).
- Cross-language config editing beyond the established TOML schema.

## Command Matrix

| Command | Description | Notes |
|---------|-------------|-------|
| `gpy theme list` | List installed themes | Reads `~/.config/gpy/themes/*.toml`, marks active theme with `*` |
| `gpy theme use <name>` | Switch active theme | Updates `config.toml`, tells agent to reload |
| `gpy theme new <name>` | Scaffold new theme file | Copies defaults, optionally opens in `$EDITOR` |
| `gpy theme export [--format fish]` | Proxy to `gpy-agent theme export` | Convenience wrapper |
| `gpy segment show <name>` | Enable a prompt segment | Updates `config.toml` `ui.enabled_segments`, reloads agent |
| `gpy segment hide <name>` | Disable a prompt segment | Removes segment from `ui.enabled_segments`, reloads agent |
| `gpy segment list` | Show enabled/available segments | Reads config + installed segment functions |
| `gpy status` | Show agent status | Thin wrapper over `gpy-agent status` |
| `gpy start/stop/restart` | Manage agent lifecycle | Combines `gpy-agent` subcommands |
| `gpy doctor` | Run validation / health checks | Wraps `gpy_config_validate` |
| `gpy config show [section]` | Print current config values | Uses `ConfigManager` to access live config |
| `gpy config set <path> <value>` | Modify specific config keys | Supports nested keys via dot syntax, triggers live reload |
| `gpy config unset <path>` | Remove optional config keys | Helpful for reverting to defaults |
| `gpy agent set <key> <value>` | Update `agent.*` settings (enabled, timeouts, etc.) | Live refresh |
| `gpy git set <key> <value>` | Update `git.*` settings | Controls git timeout, show_upstream, skip paths |
| `gpy language set <key> <value>` | Update `language.*` settings | Manage language detection toggles |
| `gpy ui set <key> <value>` | Update `ui.*` settings | Manage theme, segments, icons |

## User Experience Highlights

- **Consistent output**: colorized status, human-friendly tables for theme listing.
- **Safe writes**: `gpy config set` validates changes using the existing schema before persisting.
- **Graceful errors**: CLI returns clear messages when the agent is down or IPC fails.
- **Shell completions**: Clap-generated completions (`gpy completions <shell>`) for Fish/Bash/Zsh.

## Architecture & Implementation

1. **New binary crate**: add `bin/gpy.rs` within the existing workspace. Use `clap` for argument parsing (shared with `gpy-agent`).
2. **Internal library reuse**: extend `gpy-agent` library exports to expose:
   - Theme discovery and validation helpers.
   - Config read/write utilities.
   - Agent IPC client helpers (`status`, `reload_theme` message).
3. **Theme commands**:
   - `theme list`: list `ThemeConfig` filenames, resolve active theme via `Config`, annotate active theme with `*`.
   - `theme use`: update `Config::ui.theme`, persist via loader, call agent theme reload (IPC message).
   - `theme new`: copy from `themes/default.toml` or create minimal template; ensure theme passes validation.
4. **Segment toggles**:
   - `segment list/show/hide`: read installed segment functions, modify `Config::ui.enabled_segments`, preserve canonical order, persist, and request agent reload.
5. **Lifecycle commands**: shell out to `gpy-agent` for now; future enhancement could embed agent control APIs.
6. **Configuration commands**:
   - `config show`: pretty-print subset (e.g., `ui`, `agent`, `git`) or entire TOML.
   - `config set/unset`: support dot-paths (e.g., `ui.theme`), mutate `Config`, run `schema::validate_config`, persist, and request live reload.
   - `agent/git/language/ui set`: ergonomic wrappers around `config set` for top-level sections.
7. **Doctor command**: wrap `gpy_config_validate` function, optionally provide `--fix` flag.
8. **IPC helpers**: optionally expose a small library for CLI↔agent communication (theme reload, restart).

## Design Decisions & Rationale

- **Theme/segment reload**: After any command that mutates config (`theme use`, `segment show/hide`, `config set`), the CLI will send an IPC request to the running agent to reload config/themes. This gives immediate feedback similar to Starship/Oh-My-Posh workflows.
- **Config writes**: The CLI will expose curated helpers for documented keys (themes, segments, top-level `agent`, `git`, `language`, `ui` fields). Writes go through the existing `Config` struct + `schema::validate_config` and rewrite the TOML (comments may be lost—call this out in docs).
- **Segment discovery**: `gpy segment list` shows the current enabled list plus known built-in segments (provided via a static list in the codebase) instead of introspecting Fish functions at runtime.
- **Theme scaffolding**: `gpy theme new` clones `themes/default.toml` by default, optionally with `--force` to overwrite. Potential `--minimal` template can arrive later if requested.
- **Lifecycle commands**: v1 simply shell out to `gpy-agent` for `status/start/stop/restart` to minimize refactor. Future work can expose agent APIs if needed.
- **Feedback**: All mutating commands print success/failure only after the agent acknowledges the reload request; IPC errors are surfaced directly to the user.
- **Validation**: All config mutations run through `schema::validate_config`; errors abort the command with a clear message.
- **Permissions/backups**: No automatic backups. CLI asks for confirmation before overwriting existing theme files unless `--force`, mirroring common CLI patterns.
- **Shell scope**: Initial release targets Fish (the existing integration). Documentation should make that explicit; multi-shell support can be revisited later.
- **Packaging/docs**: Ship the `gpy` binary alongside `gpy-agent` in the same install script. Update README/GUIDES to promote `gpy` for day-to-day management while keeping `gpy-agent` documented for advanced automation.

### Prior Art

- **Starship**: `starship preset`, `starship module list`, and `starship config` provide theme/application management via CLI, reloading immediately.
- **Oh My Posh**: `oh-my-posh get themes` and related commands scaffold and switch themes quickly.
- **Tmuxp/Zellij**: CLIs mutate YAML/TOML configs using curated commands and full rewrite + validation models.

These informed the design choices above, keeping the workflow familiar to users of other prompt tooling.

## Future Extensions

- `gpy cache clear` to flush caches.
- `gpy theme edit <name>` to open theme in `$EDITOR`.
- Integration with `gpy-agent` metrics/log streaming (`gpy logs`).
- Config presets (`gpy config enable minimal`, `gpy config enable powerline`).
- Bulk config support for `config unset`, `git add-skip-path`, etc.

---

## Command Prioritization (UX Impact)

Focus initial implementation on the high-priority commands below; they deliver the most immediate UX wins.

| Priority | Command Group | Why it matters |
|----------|---------------|----------------|
| **High** | Theme list / use (`gpy theme list`, `gpy theme use <name>`) | Most frequent action; replaces manual TOML edits with a single command. |
| **High** | Segment toggles (`gpy segment show/hide/list <segment>`) | Users can customise enabled segments in seconds; mirrors `ui.enabled_segments`. |
| **High** | Agent lifecycle (`gpy status`, `gpy start`, `gpy stop`, `gpy restart`) | Core operational flow; removes need to memorise `gpy-agent` subcommands. |
| **High** | Diagnostics (`gpy doctor`) | One-command health check/fixer that wraps `gpy_config_validate`. |
| **High** | Config inspection (`gpy config show [section]`) | Safe read-only visibility into current settings builds trust. |
| **Medium** | Theme scaffolding & export (`gpy theme new`, `gpy theme export`) | Important for theme authors; moderate frequency for general users. |
| **Medium** | Config mutation (`gpy config set/unset`, section-specific setters) | Powerful knobs for advanced users; follow after core CLI is stable. |
| **Medium** | Segment discovery enhancements (`gpy segment list --available`) | Complements show/hide with discovery; nice-to-have once basics ship. |
| **Low** | Cache management (`gpy cache clear`) | Rarely needed outside debugging. |
| **Low** | Editor/log utilities (`gpy theme edit`, `gpy logs`) | Useful for power users, not essential for v1. |
| **Low** | Presets / bulk ops (`gpy config enable minimal`, etc.) | Requires curated presets; good follow-up when foundation is solid. |

---

**Next Steps**
1. Add `bin/gpy.rs` with Clap command scaffolding.
2. Expose necessary helpers from `gpy-agent` (theme discovery, config save).
3. Implement theme commands end-to-end.
4. Add lifecycle/doctor proxies.
5. Write docs/README section introducing the new CLI.
