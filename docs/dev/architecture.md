# GPY Architecture Overview

**Target Audience**: New contributors, maintainers, anyone trying to understand how GPY works

**Reading Time**: 15-20 minutes

---

## What is GPY?

GPY is a **two-process system** for fast, live-updating shell prompts across Fish, Zsh, and Bash:

```
┌──────────────────┐  gpy prompt <shell>  ┌───────────────┐                  ┌─────────────────┐
│  Shell Process   │ ── one exec per ────→│  gpy (binary) │ ←── IPC (JSON) ─→│   Rust Agent    │
│  Fish/Zsh/Bash   │       prompt         │  draws prompt │                  │ (heavy lifting) │
│  (thin wrapper)  │ ←── prompt text ─────│               │                  │                 │
└──────────────────┘                      └───────────────┘                  └─────────────────┘
       ↑                                                                             │
       └──────────────────────────── SIGURG doorbell ───────────────────────────────┘
                                    (live updates; ADR-0007)
```

**Design Philosophy**: The shell does as little as possible. Each prompt is one `gpy prompt <shell>` process that draws the whole prompt from the agent's rendered caches; a persistent background agent keeps those caches fresh and rings the shell when they change.

---

## Core Concepts

### The Shell, `gpy prompt` and the Agent

**Shell wrappers** (`bash/gpy.bash`, `zsh/gpy.zsh`, `fish/conf.d/gpy_init.fish` with `fish/functions/fish_prompt.fish`; a few hundred lines in all):
- Time each command and count commands (Bash: a `DEBUG` trap armed from `PROMPT_COMMAND`; Zsh: `preexec`/`precmd`; Fish: `fish_postexec` and `$CMD_DURATION`)
- Call `gpy prompt <shell> --status N --pid $$ --cmd COUNT [--duration-ms MS] --shell-version V` and assign its output (`PS1`, `PROMPT`, or print it from `fish_prompt`)
- Repaint on the agent's SIGURG (Zsh: `TRAPURG` + `zle reset-prompt`; Fish: a variable handler running `commandline -f force-repaint`; Bash cannot repaint an idle prompt)
- Swap in the transient line on Enter (Zsh `zle-line-finish`; Fish 4.2+ `fish_prompt --final-rendering`)
- Run `gpy prompt <shell> --exit --pid $$` when the shell exits
- Show a bare `❯ ` when the binary is missing or fails

**`gpy prompt`** (`gpy-agent/src/prompt/`):
- Reads `config.toml` and the theme on every prompt, so an edit shows at the next prompt
- Draws every built-in segment, the prompt character and the transient line in the shell's dialect (`prompt/dialect.rs`, `prompt/pill.rs`)
- Serves git and language segments from the instant prompt cache, asking the agent over its socket on a miss (`prompt/link.rs`), and does the `gpy-agent oneshot` work in process when the agent is disabled or unreachable
- Runs script segments (`prompt/script.rs`) through the embedded runner `prompt/g_runner.bash`, in one Bash 4+ child per due prompt
- Keeps the shell's lifecycle (`prompt/session.rs`): starts the agent on a shell's first prompt, registers the shell, tells the agent the working directory after a `cd`, restarts a dead agent (rate-limited and capped by `[agent.supervisor]`), consumes the agent's `.reload`/`.reregister` flags, and unregisters on `--exit`. Its state lives in `<runtime root>/refresh-throttle/<pid>.*`

**Rust Agent** (`gpy-agent/src/`):
- Runs as a background daemon (`gpy-agent start`)
- Serves IPC requests over a Unix domain socket
- Watches the filesystem for git changes and keeps the instant prompt cache fresh
- Sends SIGURG to registered shells when a rendered cache entry changed

### Request/Response Flow

Git and language segments are read from the **instant prompt cache** first. IPC is only the cold-miss, variant-mismatch and background-refresh path (see [Instant prompt cache](#instant-prompt-cache)).

```
1. gpy prompt reads <cache dir>/instant-prompts/<key>.<variant>.<token>.<ext>
     hit, fresh: use the file; no IPC
     hit, stale: use the file and send a throttled background refresh;
                 the agent repaints via SIGURG when the rendered output changed
     miss:       steps 2-4 with the cold reply budget
2. {"op":"git","cwd":…,"format":"bash-prompt"}\n over the Unix socket → gpy-agent
3. agent reuses a fresh GitStatusCache entry, or runs git status --porcelain=v2;
   it renders with the theme in the requested format and writes the instant cache file
4. ← one line of rendered text, which gpy prompt places in the prompt
```

With the agent disabled, or after a refusal (`{"error"…}` reply), `gpy prompt` does the oneshot work in process instead, at most once per prompt. Other segments (`directory`, `duration`, `character`, `clock`, `status`) are drawn in process from the theme.

**Timing**: a cache hit costs one file read and no IPC. An IPC round trip measured 0.037 ms for a ping (2026-05-15, one dev machine; `ipc_roundtrip`). These are diagnostics, not budgets; the budgets are in `benchmarks/perf-budgets.json`.

### Instant prompt cache

`InstantPromptCache` (`gpy-agent/src/cache/instant_prompt/`) holds the rendered git and language segments the daemon keeps fresh. Files live in `<cache dir>/instant-prompts/` (`paths::gpy_cache_dir`, default `~/.cache/gpy`), named `{key}.{variant}.{token}.{ext}`:

- `key`: the project root's bytes escaped into lowercase ASCII (`/` as `_s`, a capital as `_u` and its letter, a non-ASCII byte as `_x` and its hex; `path_to_cache_key`), so roots that differ only in case, Unicode normalization or bytes that are not UTF-8 never share a file on a volume that folds them; a key over 200 bytes is cut into 50-byte directory chunks
- `variant`: the segment and its position (`git`, `git_first`, `git_last`, `git_first_last`, and the same for `lang`)
- `token`: the previous segment's background colour, which the rendered chevron bakes in (`none` when there is none)
- `ext`: the dialect (`ansi`, `zsh`, `bash`)

Beside a key's language entries the agent keeps an empty presence marker, `{key}.lang.any-entry.{ext}`. The language segment's omit check tests that one file instead of globbing for any variant, since a glob reads the whole directory and slows every render as the cache grows. Git keeps no marker: it is drawn in any repository, from the cache, the agent or the oneshot. The marker is written with the entries, refreshed with them, and removed with them (the restart clear of language files, which also removes a git marker an older agent left); `prev_bg_token` never emits `-`, so no render context can name it.

Who writes and who reads:
- The agent writes a file while handling a request, again on a background refresh, on a watcher event, and for a theme or config change before it rings the reload doorbell.
- `gpy prompt` serves any file it finds. A file older than the prompt-side TTL is stale (git 5 s, `GPY_GIT_INSTANT_CACHE_TTL_SECONDS`; language 30 s, `GPY_LANGUAGE_CACHE_TTL_SECONDS`) and also triggers a refresh; a stale git file with no agent listening (no socket, or one left by a crashed agent) is replaced by the in-process oneshot. A `.none` file served for a request that has a real `prev_bg` re-queries the agent, because its chevron colour is wrong.
- Files unused for 14 days are removed by an age sweep (at agent start and hourly); files the agent cannot refresh are cleared at start. The in-memory bookkeeping is bounded (`INSTANT_PROMPT_KEY_CAPACITY`, `INSTANT_PROMPT_LAST_WRITTEN_CAPACITY`).

The benchmark side of this path is in [benchmarking.md](performance/benchmarking.md#mechanisms-behind-the-cached-prompt-budget).

---

## Multi-Shell Architecture

### One Prompt Path

All three shells share one renderer, `gpy prompt`. The dialect decides only the escaping and the output shape:

| Dialect | Output | Escaping |
|---------|--------|----------|
| `bash` | `PS1` | SGR inside `\[ \]`; `promptvars` escaping; the clock is a live `\D{…}` |
| `zsh` | `PROMPT`, 0x1F, the transient line | SGR inside `%{ %}`; `%%`; the clock is a live `%D{…}`; the wrapper doubles `!` under `prompt_bang` |
| `fish` | the prompt, 0x1F, the transient line | Plain SGR; the binary draws the clock time |

The transient line is what stays in scrollback once a line is submitted: with `ui.transient_prompt` on, the clock time (when the clock is enabled) and the prompt symbol; off with the clock, the whole prompt; off without it, nothing (the shell leaves the prompt alone).

Every data request names a `format` (`Format` in `gpy-agent/src/formatter/mod.rs`):

| Format | Used by | Reply |
|--------|---------|-------|
| `ansi` | `gpy prompt fish` | One line with ANSI SGR escapes |
| `zsh-prompt` | `gpy prompt zsh` | SGR escapes inside `%{ %}` |
| `bash-prompt` | `gpy prompt bash` | SGR escapes inside `\[ \]` |
| `json` | `gpy-agent oneshot` default and tooling | A flat object per `gpy-agent/schemas/response.json` |

`tests/bash/prompt_golden.test.bash` pins the output of all three dialects for every built-in theme (`tests/fixtures/prompt_golden/`).

### Shell-Specific Constraints

| Topic | Bash | Zsh | Fish |
|-------|------|-----|------|
| Shell options touched | `shopt -s promptvars` on every prompt | `setopt prompt_subst prompt_percent` at load; hook functions use `emulate -L zsh`; `prompt_bang` left as the user set it | None |
| Interactive guard | None: sourcing `gpy.bash` loads it | None: sourcing `gpy.zsh` loads it | `conf.d/gpy_init.fish` returns unless `status is-interactive` |
| Duration | `EPOCHREALTIME` (Bash 5+); GNU `date +%s%N` (Bash 4); whole seconds (`$SECONDS`); none on Bash 3.2 | `EPOCHREALTIME` | `$CMD_DURATION` |
| Idle repaint | None: readline cannot redraw an idle prompt; the next prompt shows fresh data | `TRAPURG` → re-run `gpy prompt`, `zle reset-prompt`; a ring during `precmd` repaints from `zle-line-init` | SIGURG handler bumps `$__gpy_repaint_trigger`; a variable handler runs `commandline -f force-repaint` |
| Transient line | None | `zle-line-finish` swaps it in | `fish_prompt --final-rendering` (Fish 4.2+) |
| Script segments | The embedded runner in one Bash 4+ child from `PATH` (else `/bin/bash`); only exported variables reach a script | Same | Same |

See [bash-limitations.md](../user/bash-limitations.md) for Bash details.

### Shared Configuration

Theme and config files are shared across shells:
```
~/.config/gpy/
  ├── config.toml    # Shared by all shells
  └── themes/        # Optional user themes (<name>.toml), selected by ui.theme
```

The config path is `$GPY_CONFIG_PATH` when set; see [Loading Hierarchy](#loading-hierarchy).

The shell code `gpy setup` installs lives apart from the config, in the data
root (`$XDG_DATA_HOME/gpy`, default `~/.local/share/gpy`):
```
~/.local/share/gpy/
  ├── fish/ zsh/ bash/   # the release layout, one tree per shell
  └── plugins/           # bundled script plugins (username, hostname)
```

### Live Updates Across Shells

The agent sends only SIGURG ([ADR-0007](adr/adr-0007-sigurg-doorbell-notifications.md)), whose default disposition is ignore, so a shell without a handler (e.g. mid-`exec`) is never killed. Meaning travels in empty flag files under `<runtime_root>/shells/`, written before the signal: `<pid>.reregister` (the agent restarted) and `<pid>.reload` (theme or config changed). The next `gpy prompt` run for that shell consumes them: it re-registers, and re-runs script segments with their circuit breakers reset. The agent rings only when its rendered cache output changed (a watcher event, or a request that wrote new content), never for a reply that carries nothing new; with `agent.live_updates = false` requests never ring, while reload and re-register rings are still sent.

When you run `git commit` in Terminal A:

```
Terminal A: git commit
      ↓
File change: .git/HEAD modified
      ↓
Agent watcher detects event
      ↓
Debouncer waits 100ms (coalesce rapid changes)
      ↓
Agent runs git status, updates cache
      ↓
Agent sends SIGURG to all registered shell PIDs in that repo
      ↓
Terminal B (Zsh/Fish): the handler re-runs gpy prompt and repaints
```

**Terminal A gets two updates**:
1. Immediate: Prompt renders after command (shows stale cache)
2. Async: ~200ms later, receives SIGURG and updates to fresh status

### Testing Multi-Shell Compatibility

What each shell supports is the [Shell feature matrix](../INSTALL.md#shell-feature-matrix).

**Test strategy**:
1. **Unit tests**: `gpy prompt` and the agent, in Rust (`cargo nextest run --lib prompt::`)
2. **Golden prompts**: `tests/bash/prompt_golden.test.bash`, every dialect and built-in theme
3. **Wrapper tests**: each shell's hooks (timing, `$?`, traps, transient line, repaint)
4. **E2E tests**: each shell on a real pseudo-terminal against a real agent; a clean-container fresh install

For reproducible shell-integration testing, see:
- [`tests/fish/README.md`](../../tests/fish/README.md)
- [`gpy-agent/tests/README.md`](../../gpy-agent/tests/README.md)
- [Testing Tiers](testing.md)

---

## System Architecture

### High-Level Component Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                      Rust Agent Process                     │
│                                                              │
│  ┌────────────┐   ┌─────────────┐   ┌──────────────────┐   │
│  │ IPC Server │   │ File Watcher│   │   Clock Timer    │   │
│  │ (tokio)    │   │  (notify)   │   │ (tokio::interval)│   │
│  └──────┬─────┘   └──────┬──────┘   └────────┬─────────┘   │
│         │                │                   │              │
│         └────────────────┴───────────────────┘              │
│                          │                                  │
│              tokio::select! event loop                      │
│                          │                                  │
│         ┌────────────────┴────────────────┐                 │
│         │                                 │                 │
│    ┌────▼─────┐                     ┌────▼──────┐          │
│    │GitStatus │                     │ClientDir  │          │
│    │  Cache   │                     │ (PIDs)    │          │
│    └──────────┘                     └───────────┘          │
└─────────────────────────────────────────────────────────────┘
                        ▲       │
                        │       │
                    JSON│       │SIGURG
                    IPC │       │doorbell
                        │       ▼
┌─────────────────────────────────────────────────────────────┐
│            Fish / Zsh / Bash Shell Process(es)              │
│                                                              │
│  prompt hook → gpy prompt <shell> → cache files / socket     │
│                                                              │
│  SIGURG handler → gpy prompt <shell> → repaint (Zsh, Fish)   │
└─────────────────────────────────────────────────────────────┘
```

### Module Dependency Graph

```
error.rs (foundation)
  ↑
  ├── config/ (TOML loading, validation)
  │
  ├── theme/ (theme loading, prompt values)
  │
  ├── security.rs (path validation)
  │
  ├── cache/ (bounded.rs, policy.rs, ttl_map.rs, instant_prompt/)
  │
  ├── git/
  │   ├── cache.rs (Git status caching)
  │   ├── status.rs (backend-agnostic repository state)
  │   └── native/
  │       ├── mod.rs (git subprocess backend)
  │       └── parser.rs (porcelain v2 parsing)
  │
  ├── language/
  │   ├── detection_cache.rs (language detection caching)
  │   ├── detector.rs (gengo-language matchers + owned ignore walk)
  │   ├── filters.rs (linguist vendor/doc exclusion globs)
  │   ├── probe_memo.rs (version probe results and recent failures)
  │   └── version.rs (version detection trait, lookup_version_at)
  │
  ├── ipc/
  │   ├── protocol/ (request parsing, PROTOCOL_VERSION)
  │   ├── wire.rs (op names, JSON reply encoding)
  │   ├── server/ (connections; HandlerRegistry::route in handlers/ maps each op to a typed handler method)
  │   ├── single_flight.rs (coalesces duplicate concurrent jobs)
  │   └── registry/ (client directory; shell_tracking.rs: tracking files, flag files, SIGURG doorbell)
  │
  └── watcher/
      ├── filesystem/mod.rs (notify integration)
      ├── debouncer.rs (event coalescing)
      ├── multi_repo/mod.rs (multi-repo coordination)
      └── hot_reload.rs (config/theme file reload polling)

agent/mod.rs (orchestrator - coordinates above)

main.rs (CLI entry point)
```

The graph is layered, not acyclic. Known cycles between modules:
- `config` and `theme`: `config/validation/{templates,theme}.rs` import `crate::theme`, while `theme/manager.rs` and `theme/model` import `crate::config`
- `git` and `cache`: `git/cache.rs` and `git/native/` use `cache::bounded` and `cache::ttl_map`, while `cache/instant_prompt/` uses `crate::git::RepositoryStatus`
- `config` and `theme` depend on `watcher::hot_reload` (`config/manager.rs`, `theme/manager.rs`), while `watcher` uses `config`

The wire type `ipc::LanguageInfo` is also the domain type: `language/display.rs` builds it, and `cache/instant_prompt` and `formatter` consume it, so those modules depend on `ipc`.

---

## Threading Model

### Agent runtime

- tokio multi-thread runtime (`Runtime::new()`, `agent/lifecycle/start.rs`).
- `Agent::start_background` (`agent/mod.rs`) runs one 5-branch `tokio::select!`:
  `server_task` (the spawned IPC server stopped), `shutdown_signal`
  (SIGINT/SIGTERM), `clock_timer` (1 s; a tick is a no-op while `agent.live_updates` is off),
  `pruning_timer` (60 s: prune dead clients, re-nudge shells) and
  `reconcile_timer` (45 s `DEFAULT_RECONCILE_INTERVAL_SECS`,
  `GPY_RECONCILE_INTERVAL_SECS` override; a tick is a no-op while `live_updates` is off).
- Git and language work runs in `spawn_blocking`.

### Watcher threads

The watchers are plain `std::thread`s: the `notify` event thread
(`watcher/filesystem/mod.rs`), the debounce flush thread, which dispatches
each expired event on its own thread (`watcher/mod.rs`), and the hot-reload
poller (`watcher/hot_reload.rs`).

### Shared State (Thread-Safe via Arc)

```rust
Arc<GitStatusCache>       // Shared: agent + IPC server
Arc<ClientDirectory>      // Shared: agent + watcher + clock timer
Arc<MultiRepoWatcher>     // Shared: agent + IPC server (for registration)
```

All shared state uses interior mutability (`Mutex` or atomic types).

### State Ownership Map

This table documents which modules own which pieces of state and how state is shared.
**RULE**: Only the owning module should mutate state. Other modules access via shared references.

| State | Owner Module | Type | Sharing Pattern | Access Rules | Examples |
|-------|-------------|------|-----------------|--------------|----------|
| **Git Status Cache** | `git::cache::GitStatusCache` | `HashMap<PathBuf, CacheEntry>` (capped at 512 repositories) | `Arc<GitStatusCache>` | • Owner: Provides `get_canonical()`, `get_any_canonical()`, `apply_full_scan()`, `update_paths_canonical()`, `invalidate_canonical()` methods<br>• Consumers: Read via `get_canonical()`, never direct mutation<br>• Thread-safe: Internal `Mutex` (all access serialized) | Allowed: `cache.get_canonical(&root)` in IPC handler<br>Allowed: `cache.apply_full_scan(&root, started, status, files)` after a scan<br>Not allowed: Accessing internal HashMap directly |
| **Client Registry** | `ipc::registry::ClientDirectory` | `HashMap<u32, ClientInfo>` | `Arc<ClientDirectory>` | • Owner: Provides `register()`, `unregister()`, `notify_*()` methods<br>• Consumers: Register clients, send signals<br>• Thread-safe: Internal `Mutex` (all access serialized) | Allowed: `registry.register(pid, cwd)` in IPC<br>Allowed: `registry.notify_repaint()` in agent<br>Not allowed: Direct PID manipulation |
| **File Watcher** | `watcher::multi_repo::MultiRepoWatcher` | `notify::RecommendedWatcher` | `Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>` | • Owner: Agent module creates and destroys<br>• Consumers: IPC server registers repos via shared slot<br>• Lazy initialization pattern | Allowed: `watcher.register(repo)` via agent<br>Allowed: Agent creates the watcher (`create_watcher` in `agent/events/reload.rs`)<br>Not allowed: Creating watcher outside agent module |
| **Configuration** | `config::manager::ConfigManager` | `Config` struct | `Arc<ConfigManager>` | • Owner: Provides `get()` (an `Arc<Config>` snapshot) and `reload_now()` methods<br>• Consumers: Read config, never cache locally<br>• Hot-reload: Config can change, always use fresh copy | Allowed: `config_mgr.get()` in handlers<br>Allowed: `config_mgr.reload_now()` to force a reload<br>Not allowed: Storing `Config` directly (stale data risk) |
| **Theme State** | `theme::manager::ThemeManager` | `ThemeConfig` | `Arc<ThemeManager>` | • Owner: Loads themes<br>• Consumers: Call `get()` for the theme<br>• Hot-reload: Theme can change, always call methods | Allowed: `let theme = theme_mgr.get()` for a shared snapshot<br>Not allowed: Caching theme colors locally |
| **Language Version Cache** | `language::version` module | Global `VERSION_MEMO` in `gpy-agent/src/language/version.rs` | `ProbeMemo<String>` (`language/probe_memo.rs`): successful probes and recent failures, each capped at `VERSION_CACHE_CAPACITY` | • Owner: `language::version` module manages global cache<br>• Consumers: Callers use `lookup_version_at(detector, cwd, policy)`; the IPC request path passes `ProbePolicy::CachedOnly` and leaves a cold version to a background probe job ([ADR-0009](adr/adr-0009-deferred-language-version-probes.md))<br>• TTL: successes expire after `language.cache_ttl_hours` (default 24); a failed probe is remembered for 1 minute (`FAILURE_TTL`)<br>• Thread-safe: `ProbeMemo` is internally synchronized | Allowed: Call `lookup_version_at()`, cache automatic<br>Allowed: Cache shared globally, TTL handles staleness<br>Not allowed: Creating separate per-request version caches<br>Not allowed: Probing a version on the IPC request path |
| **IPC Server State** | `ipc::server::EndpointHandle` | Socket listener | Owned by `Agent` | • Owner: Agent creates and holds server<br>• Lifecycle: Lives until agent shutdown<br>• Thread-safe: Tokio runtime handles connections | Allowed: Agent creates in `Agent::new()`<br>Allowed: Server runs in `Agent::start_background()`<br>Not allowed: Multiple server instances |

#### State Access Patterns

**Correct Pattern**: Access state via owner's public API
```rust
// CORRECT: Use owner's API
let status = git_cache.get_canonical(&repo_root);  // GitStatusCache::get_canonical()
clients.notify_repaint(None);                      // ClientDirectory::notify_repaint()

// CORRECT: Agent coordinates state updates (agent/events/mod.rs)
fn handle_file_event(ctx: &AgentContext, event: &DebouncedEvent, config: &Config) {
    // Merge a pathspec scan with GitStatusCache::update_paths_canonical, or
    // publish a full scan with GitStatusCache::apply_full_scan, then notify
    // clients.
}
```

**Incorrect Pattern**: Bypassing abstractions or duplicating state
```rust
// WRONG: Accessing internal state directly
let internal_map = cache.inner.lock().unwrap();  // Breaks encapsulation

// WRONG: Duplicating state in another module
pub struct MyModule {
    cached_config: Config,  // Stale data! Use Arc<ConfigManager> instead
}

// WRONG: Cross-module coordination outside agent
impl GitCache {
    fn refresh(&self, repo: &Path, clients: &ClientDirectory) {
        // ...
        clients.notify_repaint(None);  // Coordination belongs in agent module!
    }
}
```

#### Arc Sharing Guidelines

When adding new shared state:

1. **Identify owner**: Which module has primary responsibility?
2. **Design API**: What operations should be exposed? (get, set, invalidate, etc.)
3. **Choose pattern**:
   - `Arc<T>` for immutable shared state
   - `Arc<Mutex<T>>` for mutable shared state (coarse locking)
   - `Arc<RwLock<T>>` for read-heavy state (fine-grained locking)
   - `Arc<Mutex<Option<Arc<T>>>>` for lazy/optional initialization
4. **Document in table**: Add row to this State Ownership Map
5. **Add to agent**: Agent module coordinates initialization and cross-module access
6. **Avoid duplication**: Never cache shared state locally (risk of stale data)

#### Common State Management Mistakes

| Mistake | Problem | Solution |
|---------|---------|----------|
| Storing `Config` directly | Misses hot-reload updates | Use `Arc<ConfigManager>`, call `get()` |
| Caching theme colors | Stale colors after theme reload | Query `ThemeManager` each time |
| Creating multiple watchers | Resource leak, duplicate events | Single watcher in agent, shared via slot |
| Direct HashMap access | Breaks thread safety, violates ownership | Use owner's public methods |
| Cross-module coordination | Circular dependencies, unclear flow | All coordination in agent module |

#### State Lifecycle

```text
┌──────────────────────────────────────────────────────┐
│  Agent::new() - Initialization Phase                 │
├──────────────────────────────────────────────────────┤
│  1. Create GitStatusCache::new()                     │
│  2. Create ClientDirectory::new()                    │
│  3. Create ConfigManager::with_defaults()            │
│  4. Create ThemeManager::new()                       │
│  5. Create MultiRepoWatcher (if enabled)             │
│  6. Wrap each in Arc<T> for sharing                  │
│  7. Pass Arc references to server/timers             │
└──────────────────────────────────────────────────────┘
                        ↓
┌──────────────────────────────────────────────────────┐
│  Agent::start_background() - Runtime Phase           │
├──────────────────────────────────────────────────────┤
│  • IPC requests read/write state via Arc references  │
│  • File events trigger coordinated state updates     │
│  • Config reloads invalidate dependent caches        │
│  • All state mutations logged for debugging          │
└──────────────────────────────────────────────────────┘
                        ↓
┌──────────────────────────────────────────────────────┐
│  Agent Drop - Cleanup Phase                          │
├──────────────────────────────────────────────────────┤
│  1. Watcher stopped (drop MultiRepoWatcher)          │
│  2. Server stopped (drop EndpointHandle)             │
│  3. Arc reference counts drop to zero                │
│  4. State destructors run automatically              │
│  5. Socket file removed                              │
└──────────────────────────────────────────────────────┘
```

Rust ownership cleans up state automatically; only watchers and servers need an explicit stop.

---

## Key Data Flows

### Git Status Request

```
gpy prompt fish: the git segment
  ↓
read <cache dir>/instant-prompts/<key>.git….ansi
  ↓
hit (fresh or stale): use the file; a stale hit also sends a throttled
background refresh (steps below, reply discarded). Only a miss continues.
  ↓
prompt/link.rs: send the request, with the cold reply budget
  ↓
JSON: {"op":"git","cwd":"/path","format":"ansi"}
  ↓
Unix socket → gpy-agent IPC server
  ↓
Message::RepositoryStatus parsed
  ↓
security::validate_path(path) → reject if suspicious
  ↓
git_cache.get_canonical(git_root) → check freshness
  ↓
if stale:
    git::status::load_repository_state(path, max_ahead_behind, stash_enabled, timeout)
      ↓
    native backend: git status --porcelain=v2 --branch
      ↓
    Parse output, build RepositoryStatus
      ↓
    git_cache.apply_full_scan(git_root, started, status, files)
  ↓
Render response in ansi format:
  - Apply theme colors
  - Generate ANSI escape codes
  - Add delimiters (if not is_last)
  - Write the instant cache file
  ↓
← ansi: "\x1b[32m main \x1b[0m  2 \x1b[33m⚡1\x1b[0m"
  ↓
gpy prompt places the text in the prompt it prints
```

A file-event refresh instead runs one pathspec `git status --porcelain=v2 -z -- <paths>` (`get_pathspec_status`) and merges the result into the cached status.

### File Watcher Event

```
User: git commit
  ↓
File change: .git/HEAD
  ↓
notify crate: RawEvent { path, kind: Modify }
  ↓
filesystem::FileSystemWatcher filters:
  - Ignore: .git/objects/, .git/logs/
  - Accept: .git/HEAD, .git/refs/, .git/index
  ↓
PendingEvent { event: Git, repo: /path }
  ↓
debouncer::DebounceEngine:
  - Check HashMap for existing event
  - If exists: reset expiry to now + 100ms, and merge Git paths into the
    pending event (bounded; past GitPaths::MAX_PATHS it degrades to a
    whole-repo full scan)
  - If new: create entry with expiry
  ↓
(100ms passes)
  ↓
Flush thread wakes, finds expired event
  ↓
DebouncedEvent → callback
  ↓
agent::events::handle_file_event():
  - Check if cache is fresh (cooldown window)
  - If stale: run git status, update cache
  - Ring registered clients with SIGURG only if the agent's rendered cache output changed
  ↓
Shells receive SIGURG (no flag file)
  ↓
Zsh: TRAPURG re-runs gpy prompt, zle reset-prompt
Fish: SIGURG handler bumps $__gpy_repaint_trigger → commandline -f force-repaint
  ↓
Prompt redraws with the fresh cache entry
```

### Clock Timer

```
1 s tick (clock_timer; no-op while agent.live_updates is off)
  ↓
handle_clock_tick() → skip unless the clock segment is enabled
  ↓
clock_signal_decision(): ring every tick if show_seconds, else once per minute
  ↓
client_registry.notify_repaint(None)
  ↓
SIGURG to every registered shell
```

Most user-visible behaviors follow one pattern: filesystem
event → watcher → agent cache update → SIGURG (plus a `<pid>.reload` flag for
config/theme) → shell repaint. `agent/mod.rs` and `agent/events/` coordinate it.

---

## IPC Protocol Specification

GPY uses a **JSON-based request/response protocol** over Unix domain sockets. The protocol is **shell-agnostic** - any shell that can write JSON and read from a socket can communicate with the agent.

### Transport Layer

**Socket Location** (`paths::SocketPath`, `paths::runtime_root_for`):
- Override: `--socket <path>` on `gpy-agent start`/`stop`/`status`/`config reload`, or `$GPY_AGENT_SOCKET_PATH` (empty counts as unset)
- Default: `<runtime root>/gpy.sock`, where the runtime root is the first of `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy` and `$HOME/.cache/gpy`; relative `XDG_*` values are ignored. There is no `/tmp/gpy` fallback: with none usable, `gpy-agent start` exits 1 and `status` exits 4 (a `--socket` or `GPY_AGENT_SOCKET_PATH` still works). The config root (`$XDG_CONFIG_HOME/gpy` or `~/.config/gpy`) and data root (`$XDG_DATA_HOME/gpy` or `~/.local/share/gpy`) have no relative fallback either
- There is no `config.toml` key. `gpy prompt` resolves the same path as the agent; `gpy debug paths` prints it
- Only `gpy-agent start` creates the default runtime directory (`agent::lifecycle::ensure_runtime_dir`); other commands only resolve the path
- The path must fit the Unix socket limit (103 bytes on macOS, 107 on Linux); `gpy-agent` refuses a longer one

**Connection Type**: SOCK_STREAM (stream socket, not datagram)

**Protocol**: Line-delimited
- Each request is a single JSON object terminated by newline (`\n`)
- Each reply is one newline-terminated line. For the prompt formats (`ansi`, `zsh-prompt`, `bash-prompt`) it is the rendered segment text; for `json` requests and control ops it is a JSON object
- Multiple requests can be sent over the same connection (connection pooling)

**Refusals**: a request the agent refuses (bad JSON, unknown op, missing field, a denylisted or invalid path) gets a single `{"error":"Parse error: ..."}` line in every format, never an empty line. `gpy prompt` treats any reply that starts `{"error"` as a permanent refusal and does the oneshot work in process instead. An empty line still means the agent is alive but late: the segment is omitted and no oneshot work is done.

**Security**:
- The socket is chmod'ed to `0600` right after bind
- The socket's directory is trusted only if it is a real directory (not a symlink) owned by the user. An owned group- or world-writable directory is set to `0700` when it is the default runtime root and refused when it comes from `--socket` or `$GPY_AGENT_SOCKET_PATH`. `gpy-agent start` checks before it touches the directory, and the daemon checks again immediately before bind (`secure_socket_dir` in `ipc/server/socket_dir.rs`)
- No handshake or peer-credential check: any process running as the user can connect. See [security-model.md](security-model.md) for the full trust policy

### Operations

Each request is one newline-terminated JSON object with an `op`:

`git` `lang` `directory` `clock` `duration` `character`
`exit_status` `ping` `status` `shutdown` `config_reload` `register` `unregister` `workspace`

`exit_status` renders the status segment's pill from the previous command's exit
status; `status` is the agent health check.

The segment ops that draw a pill (`git`, `lang`, `directory`, `clock`, `duration`,
`exit_status`) all carry `is_last`, `is_first` and
`prev_bg`, so the agent can drop the opening cap of the first pill and shape the
closing cap of the last. A theme without a `format` for `clock` or `status` gets
the shell's local pill instead, which draws the same
leading space; so does a themed pill when the agent gives no reply (no oneshot
fork for these), and for `status` when it replies
`{"error":...}` (an older agent without `exit_status`).

Per-op fields, `format` values and reply shapes:
[`gpy-agent/schemas/README.md`](../../gpy-agent/schemas/README.md).

### Protocol Versioning

`PROTOCOL_VERSION` (`gpy-agent/src/ipc/protocol/mod.rs`); shells compare for
equality and treat a mismatch as an unreachable agent, and `gpy-agent start`
replaces a running agent that reports another protocol. There is no negotiation.

### Performance Characteristics

A git or language segment with a fresh or stale instant-cache file costs a file read and no IPC. IPC runs on a cold miss, a variant mismatch or a background refresh: `benchmarks/perf-budgets.json` sets the `ipc_latency` budget at 1 ms target, 1.5 ms max (the gates add the 15% `measurement_variance_percent` to each max); a ping measured 0.037 ms on a dev machine, 2026-05-15. A fresh query adds a `git status --porcelain=v2` subprocess; the per-repo-size figures live in the module docs at the top of `gpy-agent/src/git/native/mod.rs`. A fresh `GitStatusCache` entry returns without spawning git at all.

### Security Considerations

See [security-model.md](security-model.md) for the trust boundaries, threat model and defenses.

---

## Design Decisions

### Why Unix Domain Sockets?

**Alternatives considered**:
- Named pipes: Not bidirectional, awkward for request/response
- Shared memory: Complex synchronization, no natural request boundaries
- HTTP: Overkill, adds dependency on web server

**Chosen**: Unix sockets
- Fast (< 1ms latency)
- Secure (filesystem permissions)
- Bidirectional, stream-based
- Standard library support

### Why the git CLI Instead of gix?

GPY started on `gix` (Gitoxide) and moved to a `git status --porcelain=v2 --branch`
subprocess. ADR-0003 records the original decision and its reversal;
[design-decisions.md](design-decisions.md) carries the full rationale.

**gix library** (native Rust):
- In-process, no spawning, type-safe API
- But no feature parity for sparse checkouts or submodules, which real repos hit

**git CLI** (chosen):
- Inherits git's own optimizations: `core.untrackedCache` reduces a full scan to
  a check of what git already knows changed. Every command runs with `core.fsmonitor` off, so a repository's config cannot make a prompt run its fsmonitor hook; a repository-local `filter.<driver>.clean` or `process` command still runs when `git status` compares a filtered file's content (see [security-model.md](security-model.md#subprocesses))
- One subprocess returns branch, ahead/behind and file status together
- `--porcelain=v2` is machine-readable and has been stable since Git 2.11.0 (2016)
- Spawn overhead is real but small against what the untracked cache saves:
  see the `git_status_cold` budget in `benchmarks/perf-budgets.json`

### Why Separate Debouncing and Cache Cooldown?

**Debouncing** (100ms): Prevents redundant signals to Fish processes
- Problem solved: `git commit` writes 5 files → only 1 signal after quiet period

**Cache Cooldown** (150ms): Prevents redundant git queries
- Problem solved: File event fires before IPC request completes → skip re-query

### Why tokio Instead of Threads?

**Async needed for**:
- IPC server (many concurrent connections)
- Timers (clock updates)
- Signal handling (shutdown)

**Threads used for**:
- File watcher flush (periodic, not event-driven)

---

## Security Model

See [security-model.md](security-model.md) for the trust boundaries, threat model and defenses.

---

## Performance Characteristics

### Latency Targets

| Operation | Target | Baseline | Notes |
|-----------|--------|---------|-------|
| IPC round-trip (`ipc_latency`) | 1 ms (max 1.5 ms) | 0.037 ms (`ipc_roundtrip`) | A ping; the cold-miss and refresh path |
| Git status, cold (`git_status_cold`) | 500 ms (max 750 ms) | — | First query in a repository |
| Prompt render, warm (`prompt_render`) | 20 ms (max 50 ms) | — | Agent up, instant cache fresh |
| Prompt render, cold cache (`prompt_render_cold`) | 150 ms (max 300 ms) | — | Instant cache cleared before every render |
| Prompt render, stale (`prompt_render_stale`) | 40 ms (max 100 ms) | — | Serve-stale path |
| Prompt render, agent down (`prompt_render_degraded`) | 80 ms (max 200 ms) | — | Oneshot fallback |

The gates enforce `max` plus the 15% `measurement_variance_percent`, so a 1.5 ms max fails above 1.725 ms; `target` is never enforced. The per-shell render and script-segment budgets are in `benchmarks/perf-budgets.json`.

### Memory Footprint

- Agent resident memory budget (`scripts/ci-bench.sh`): `memory_usage` 15 MB target, 50 MB max at startup; `memory_long_running` 20 MB target, 50 MB max peak under sustained load
- Every path-keyed cache is bounded by entry count, and git file detail by a byte budget (below)

### Caching Strategy

**Git status cache** (`git/cache.rs`, `cache/policy.rs`, `cache/bounded.rs`):
- TTL: 30 s from the last full scan (`CachePolicy::git_status`), which catches changes the watcher cannot see. A hit within the 150 ms cooldown (`GIT_STATUS_COOLDOWN`) after any update skips background revalidation
- Unwatched repositories: a status scanned while the repository was already unwatched (no live watch, `agent.live_updates = false`, or the watcher failed to start) is reused for up to 2 s (`UNWATCHED_TRUST_TTL`, `get_canonical_unwatched`) instead of being rescanned every prompt. A status scanned under a live watch is never reused after the watch ends, and an invalidation or full-scan change event ends reuse at once
- Updates: file events merge a pathspec scan into the entry, or replace it with a full scan
- Eviction: least recently updated entries beyond `GIT_STATUS_CACHE_CAPACITY` (512 repositories); per-file detail beyond `GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES` (10 MiB, about 80 B per dirty file) is dropped oldest first, and such an entry needs a full scan before it takes another pathspec merge

**Language detection cache** (`language/detection_cache.rs`):
- Refresh interval: 30 s (`LANGUAGE_REFRESH_INTERVAL`); an entry whose git-status signature is unchanged is still re-detected after 5 minutes (`LANGUAGE_SIGNATURE_MAX_AGE`)
- Eviction: least recently refreshed entries beyond `DETECTION_CACHE_CAPACITY` (512 directories), through `evict_to_capacity`
- `language.cache_ttl_hours` (default 24, 1–720) governs only the version, mise and virtualenv caches, not detection. The version cache holds at most `VERSION_CACHE_CAPACITY` (1024) successes and as many failures; a failed probe is remembered for 1 minute

**Instant prompt cache**: see [Instant prompt cache](#instant-prompt-cache) for freshness, the 14-day age sweep and the in-memory bounds.

---

## Configuration System

### Loading Hierarchy

```
1. $GPY_CONFIG_PATH (when set and non-empty, the ONLY config file: read and created there even if absent;
                     a relative value is ignored with a warning; empty counts as unset)
2. $XDG_CONFIG_HOME/gpy/config.toml
3. ~/.config/gpy/config.toml
4. Built-in defaults (config::Config::default())
```

Without `GPY_CONFIG_PATH` the first existing file of 2 and 3 wins (`config/location.rs`). A theme is not a separate file in this chain: `ui.theme` names a theme, resolved user (`~/.config/gpy/themes/<name>.toml`), then plugin, then built-in (`default`, `text`, `starship`).

### Config Structure

```toml
[agent]
enabled = true
timeout_seconds = 5

[git]
timeout_seconds = 10
skip_paths = ["/mnt/slow-drive"]

[language]
cache_ttl_hours = 24

[ui]
show_icons = true
```

Every key is in the [Configuration Reference](../user/configuration-reference.md). The socket path is not configurable here; see [Transport Layer](#transport-layer).

### Hot-Reload Status

**Applied by a running agent** (no restart required):
- Theme changes (colors, glyphs, segment styling) and palette changes
- Config changes: `ui.enabled_segments`, `git.timeout_seconds` (read on each scan), `language.cache_ttl_hours` (applied by `handle_config_reload`), watch settings
- Git status updates (cross-terminal via file watcher)
- Clock updates (every minute, or every second with `show_seconds`)

A config file that exists but does not load keeps the last good configuration.

### Hot-Reload Implementation

**Critical Design Decisions**:

1. **Unified Signal Path**: Repaints (git/clock) and reloads (config/theme) arrive as the same SIGURG; a reload is a SIGURG preceded by a `<pid>.reload` flag file:
   - Agent sends signal
   - Fish increments `__gpy_repaint_trigger`; a variable change handler calls `force-repaint` (no direct `commandline -f` calls from signal handlers: they cause timing issues)
   - Zsh re-runs `gpy prompt` from `TRAPURG` and calls `zle reset-prompt`
   - The next `gpy prompt` consumes the flag

2. **Agent Must Send a Reload for Every Rendering Config Change**: `gpy prompt` reads `config.toml` on every prompt, so a reload ring exists to repaint idle prompts and to reset script-segment circuit breakers; the agent sends it for every config change that affects rendering, including one that only disables a segment. Settings that render nothing (`agent.timeout_seconds`, `agent.supervisor`) ring nothing.

3. **Avoid Shell-Side Config Detection**: Don't use `--on-event fish_prompt` (or a shell hook) to detect config file changes - it creates race conditions with the agent's file watcher. Let the agent be the single source of truth for config changes.

**Common Pitfalls**:
- Using `commandline -f execute` for repaints → creates blank prompt lines
- Multiple signal handlers → causes double-repaints
- Forgetting the reload notification in disabled-feature paths → Fish keeps stale segment list

**Test Coverage**:

Hot-reload tests:

1. **Reload Notification Delivery** (`config_hot_reload_tests.rs`):
   - Reload notification sent when disabling git
   - Reload notification sent when enabling git
   - Edge case with multiple feature toggles

2. **Config Watcher** (`config_hot_reload_tests.rs`):
   - `test_config_watcher_triggers_reload_on_git_toggle` - File watcher detects changes

These tests cover the bug where unlisting `git` from `ui.enabled_segments` sent no reload notification, so Fish kept stale segment lists and wrong delimiters.

---

## Testing Strategy

### Test Pyramid

```
        ╱╲
       ╱E2E╲       E2E: agent_daemon_tests.rs, e2e_ipc_tests.rs
      ╱────╲
     ╱ Integ╲      Integration: language_tests.rs, git_status_tests.rs
    ╱────────╲
   ╱   Unit   ╲    Unit: `#[cfg(test)]` modules and `*_tests.rs` files beside the code
  ╱────────────╲
```

### Test Files

**Rust** (gpy-agent/tests/):
- `e2e_ipc_tests.rs`: Full request/response cycles with real agent
- `agent_daemon_tests.rs`: Agent lifecycle, signal handling
- `config_hot_reload_tests.rs`: Reload notification delivery, config watcher
- `git_status_tests.rs`: Git detection, caching, error cases
- `language_tests.rs`: Language detection, versioning
- `watcher_tests.rs`: Debouncing, multi-repo coordination
- `clock_timer_tests.rs`: Timer alignment, signal delivery

**Fish** (tests/fish/):
- `e2e_live_updates_signal.test.fish`: Repaint signal delivery, agent restart
- `e2e_interactive_session.test.fish`: Real `fish -i` on a pty: idle repaint, registration, config reload
- `doorbell_signal.test.fish`: SIGURG handler, flag files, repaint trigger

### Coverage Goals

- **Unit tests**: 80%+ coverage
- **Integration**: All major code paths
- **E2E**: Critical user scenarios (git commit, clock updates, agent restart)

### Running Tests

```bash
# Rust tests (unit + integration)
cargo test --quiet

# Strict linting (from the repo root)
just lint

# All shell suites (repo, Fish, Bash, Zsh)
just test-shell

# Fish integration tests
./scripts/test-fish.sh

# Interactive sessions on a real pseudo-terminal (idle repaint, two shells,
# a config reload reaching a Fish client)
fish tests/fish/e2e_interactive_session.test.fish
```

---

## Common Operations

### Adding a New Segment

See [add-new-segment.md](add-new-segment.md) for the checklist.

Route A, a bundled script plugin under `plugins/<id>/`, is the default and needs no Rust change. Route B, an agent-rendered segment, needs an issue and maintainer approval first.

An agent-rendered segment is drawn by `gpy prompt` for all three shells:
1. Render it in the agent (`ipc/`, `formatter/`), and draw it in `gpy-agent/src/prompt/mod.rs` (`would_render`, `segment`)
2. The segment must be in `ui.enabled_segments`
3. Add golden cases to `tests/bash/prompt_golden.test.bash` (`GPY_UPDATE_GOLDEN=1` regenerates them; review the diff)
4. Test with both agent and oneshot modes

### Modifying IPC Protocol

**CRITICAL**: Protocol changes break compatibility

Steps:
1. Add the op to `Op::ALL` in `gpy-agent/src/ipc/wire.rs` (the only list of op names) and the `Message` enum in `gpy-agent/src/ipc/mod.rs`
2. Handle it in the `into_message` dispatch (`ipc/protocol/mod.rs`) and route it in `HandlerRegistry::route` (`ipc/handlers/mod.rs`)
3. Update the schemas in `gpy-agent/schemas/` (`message.json`, `response.json`); `gpy-agent/tests/schema_validation_tests.rs` checks them against the code
4. Update `gpy-agent/src/prompt/link.rs` if `gpy prompt` sends the op
5. Bump `PROTOCOL_VERSION` if the change is incompatible (`gpy prompt` and the agent ship together; there is no negotiation)
6. Coordinate shell and Rust deployment

### Debugging Cache Issues

Enable debug logging:
```bash
GPY_DEBUG_LOG=/tmp/gpy-debug.log gpy start
tail -f /tmp/gpy-debug.log
```

Look for:
- Cache hits/misses
- Cooldown window checks
- File watcher events
- Signal delivery

### Performance Profiling

```bash
# Check agent status
gpy status

# Run diagnostics
gpy doctor

# IPC latency (socket path from `gpy debug paths`)
socat -T 1 - UNIX-CONNECT:"$XDG_RUNTIME_DIR/gpy/gpy.sock" <<< '{"op":"ping"}'

# Git query timing
time git status --porcelain
```

---

## Next Steps for New Contributors

1. **Read this doc**
2. **Read module docs**: `gpy-agent/src/agent/mod.rs`, `gpy-agent/src/watcher/mod.rs`
3. **Trace a request**: Follow git status request through entire stack
4. **Run tests**: `just test`, `cargo test`, `./scripts/test-fish.sh`
5. **Read [add-new-segment.md](add-new-segment.md)**: Learn how to add segments
6. **Pick a good first issue**: Look for `good-first-issue` label

### Good First Issues

- Add new language detector (see `language/version.rs`)
- Add new segment (e.g., Docker, Kubernetes context)
- Improve error messages with more context
- Add config validation warnings
- Write more integration tests

### Advanced Topics

After you're comfortable with basics:
- Modify watcher debouncing algorithm
- Optimize cache eviction strategy
- Performance optimization

---

## Further Reading

- [add-new-segment.md](add-new-segment.md) - How to add segments
- [ADR-0004: SIGUSR1 Live Updates](adr/adr-0004-sigusr1-live-updates.md) - Why the watcher signals shells instead of polling
- [ADR-0007: SIGURG Doorbell](adr/adr-0007-sigurg-doorbell-notifications.md) - The current single-signal protocol and flag files
- [CONTRIBUTING.md § Quality Gates](../../CONTRIBUTING.md#quality-gates) - How to run every suite
