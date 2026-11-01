# Adding a First-Party Segment

How to add a new first-party segment to GPY. To create a third-party community segment, see [Build a Segment Plugin](build-segment-plugin.md).

## 1. Decide where the data comes from

| Data source | Approach |
|-------------|----------|
| Environment variable | Shell-only |
| File existence check | Shell-only |
| Command output under ~10 ms | Shell-only |
| Slower command, external API, complex parsing | Agent-assisted |

A shell-only segment runs synchronously on prompt render, so everything it does counts against the prompt budget (`prompt_render`: 20 ms target, 50 ms max in `benchmarks/perf-budgets.json`).

## 2. Implementation Routes

There are two routes for introducing a new first-party segment:

### Route A: Bundled Script Plugin (Default)

The default route is a bundled script plugin under `plugins/<id>/`, modelled on `plugins/hostname` and `plugins/username`.

Bundled script plugins run across Bash, Zsh, and Fish without modifying the core Rust agent binary. `gpy setup` automatically installs bundled plugins into the data directory (`$XDG_DATA_HOME/gpy/plugins`).

#### Plugin Structure

Create the plugin directory under `plugins/<id>/`:
```text
plugins/<id>/
├── plugin.toml
├── segments/
│   └── <id>.toml
└── bin/
    └── <id>.sh
```

#### Packaging and Release Checklists

When adding a bundled script plugin, add the new files to all expected-entry lists in packaging and tests:
1. `tests/repo/release_packaging.test.bash` (the plugin tar/zip entries and the shell-files archive check)
2. `scripts/package-release.sh` (archive entry list and `SHELL_FILES_REQUIRED`)

Run `git grep -n 'plugins/username/plugin.toml' -- scripts tests` to find all four locations.

### Route B: Agent-Rendered Segment (Rust resolver plus IPC)

Segments that require direct Rust performance, low-level system APIs, or complex daemon caching (such as `git` or `language`) can be implemented as agent-rendered segments.

Because this route modifies the IPC `Message` enum, schema protocols, and agent resolvers, **it requires an issue and maintainer approval first**.

## 3. Enable It

Add the segment name to `ui.enabled_segments` in `config.toml`:

```toml
[ui]
enabled_segments = ["clock", "duration", "language", "directory", "<id>", "git"]
```

## 4. Test

1. The segment appears when active.
2. It disappears when inactive or when required commands/files are missing.
3. Colors and icons render correctly in your theme.
4. Shell prompt render stays within prompt latency budgets.

| Problem | Cause | Fix |
|---------|-------|-----|
| Segment not appearing | Script exited non-zero or empty, or name not in `ui.enabled_segments` | Test script directly and check `ui.enabled_segments` |
| Wrong colors | Theme not loaded | `gpy-agent theme export --shell fish \| source` |
| Slow prompt | Expensive detection | Cache with `ttl_secs` or move work into daemon |

## Related

- [Build a Segment Plugin](build-segment-plugin.md) - Plugin authoring guide
- [Plugin Framework](plugins.md) - Plugin discovery and execution contract
- [Benchmarking Methodology](performance/benchmarking.md) - Prompt performance and budgets
