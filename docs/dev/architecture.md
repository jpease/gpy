# GPY Architecture Overview

**Target Audience**: New contributors, maintainers, anyone trying to understand how GPY works

**Reading Time**: 15-20 minutes

---

## What is GPY?

GPY is a **two-process system** for creating fast, live-updating shell prompts across Fish, Zsh, and Bash:

```
┌───────────────────┐                  ┌──────────────────┐
│  Shell Process    │ ←── IPC (JSON) ─→│   Rust Agent     │
│  Fish/Zsh/Bash    │                  │  (heavy lifting) │
│  (lightweight)    │                  │                  │
└───────────────────┘                  └──────────────────┘
       ↑                                        │
       │                                        │
       └─────── SIGURG doorbell ────────────────┘
              (live updates; ADR-0007)
```

**Design Philosophy**: Keep the shell fast by offloading work to a persistent background agent. The agent provides a shell-agnostic IPC interface, allowing Fish, Zsh, and Bash to share the same backend.

---

## Core Concepts

### The Agent-Fish Split

**Fish Shell** (`fish/segments/`, `fish/core/`, `fish/functions/`):
- Renders the prompt UI (ANSI codes, delimiters)
- Makes IPC requests to agent for git/language data
- Handles the agent's SIGURG doorbell to repaint, reload or re-register
- Falls back to oneshot mode if agent unavailable

**Rust Agent** (`gpy-agent/src/`):
- Runs as background daemon (started via `gpy start`)
- Serves IPC requests over Unix domain socket
- Watches filesystem for git changes
- Sends SIGURG to shell processes for live updates
- Maintains caches to avoid redundant git queries

**Why this split?**
- Fish shell builtins are slow for complex operations (git status, directory traversal)
- Persistent agent can cache results and maintain file watchers
- IPC latency < 1ms, much faster than spawning git processes

### Request/Response Flow

Git and language segments are read from the **instant prompt cache** first. IPC is only the cold-miss, variant-mismatch and background-refresh path (see [Instant prompt cache](#instant-prompt-cache)).

```
1. segment_git_render (Fish) / __gpy_segment_git (Zsh, Bash)
2. __gpy_read_instant_cache reads <cache dir>/instant-prompts/<key>.<variant>.<token>.<ext>
     hit, fresh: print the file; no IPC
     hit, stale: print the file and send a throttled background refresh (__gpy_maybe_refresh);
                 the agent repaints via SIGURG when the rendered output changed
     miss:       steps 3-6 with the cold reply budget (GPY_IPC_COLD_TIMEOUT_MS, Fish default 500 ms)
3. Fish:     __gpy_build_data_payload … ansi → __gpy_ipc_send
   Zsh/Bash: __gpy_build_data_request_into … zsh-prompt|bash-prompt → __gpy_send_json
4. {"op":"git","cwd":…,"format":"ansi"}\n over the Unix socket → gpy-agent
5. agent reuses a fresh GitStatusCache entry, or runs git status --porcelain=v2;
   it renders with the theme in the requested format and writes the instant cache file
6. ← one line of rendered text, which the shell prints
```

With the agent disabled, or after a refusal (`{"error"…}` reply), the shell runs `gpy-agent oneshot` instead, at most once per prompt render (`__gpy_oneshot_claim`). Other agent-rendered segments (`directory`, `duration`, `character`, `clock`, `exit_status`) have no cache file: each is one IPC request per render unless the shell's memo of the previous render still applies.

**Timing**: a cache hit costs one file read and no IPC. An IPC round trip measured 0.037 ms for a ping (2026-05-15, one dev machine; `ipc_roundtrip`). These are diagnostics, not budgets; the budgets are in `benchmarks/perf-budgets.json`.

### Instant prompt cache

`InstantPromptCache` (`gpy-agent/src/cache/instant_prompt/`) holds the rendered git and language segments the daemon keeps fresh. Files live in `<cache dir>/instant-prompts/` (`paths::gpy_cache_dir`, default `~/.cache/gpy`), named `{key}.{variant}.{token}.{ext}`:

- `key`: the project root path, chunked into directories when long
- `variant`: the segment and its position (`git`, `git_first`, `git_last`, `git_first_last`, and the same for `lang`)
- `token`: the previous segment's background colour, which the rendered chevron bakes in (`none` when there is none)
- `ext`: the dialect (`ansi`, `zsh`, `bash`)

Who writes and who reads:
- The agent writes a file while handling a request, again on a background refresh, on a watcher event, and for a theme or config change before it rings the reload doorbell.
- A shell serves any file it finds. A file older than the shell-side TTL is stale (git 5 s, `GPY_GIT_INSTANT_CACHE_TTL_SECONDS`; language 30 s, `GPY_LANGUAGE_CACHE_TTL_SECONDS`) and also triggers a refresh. A `.none` file served for a request that has a real `prev_bg` re-queries the agent, because its chevron colour is wrong.
- Files unused for 14 days are removed by an age sweep (at agent start and hourly); files the agent cannot refresh are cleared at start. The in-memory bookkeeping is bounded (`INSTANT_PROMPT_KEY_CAPACITY`, `INSTANT_PROMPT_LAST_WRITTEN_CAPACITY`).

The benchmark side of this path (`prompt_render`, `prompt_render_cold`, `prompt_render_stale`, `prompt_render_degraded`) is in [benchmarking.md](performance/benchmarking.md#mechanisms-behind-the-cached-prompt-budget).

---

## Multi-Shell Architecture

### Design Principles

GPY supports Fish, Zsh, and Bash through a **shell-agnostic backend** with **shell-specific frontends**:

```
┌─────────────────────────────────────────────────────────────┐
│                    Shell-Agnostic Layer                     │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │           Rust Agent (gpy-agent)                     │   │
│  │  • IPC Server (JSON protocol)                        │   │
│  │  • Git Status Cache                                  │   │
│  │  • Language Detection                                │   │
│  │  • File Watcher                                      │   │
│  │  • Theme Management                                  │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                            ▲
                            │ JSON IPC (shell-agnostic)
                            │
        ┌───────────────────┼───────────────────┐
        │                   │                   │
┌───────▼──────┐    ┌───────▼──────┐    ┌──────▼───────┐
│  Fish Shell  │    │  Zsh Shell   │    │  Bash Shell  │
│              │    │              │    │              │
│ fish/        │    │ zsh/         │    │ bash/        │
│ • core/      │    │ • core/      │    │ • core/      │
│ • segments/  │    │ • segments/  │    │ • segments/  │
│ • functions/ │    │ • gpy.zsh    │    │ • gpy.bash   │
└──────────────┘    └──────────────┘    └──────────────┘
```

**Key Insight**: The agent doesn't know or care which shell it's talking to. All shell-specific rendering (ANSI codes, prompt syntax) is handled by the shell integration.

### Shell Integration Layers

Each shell implementation has three layers:

#### Layer 1: Core IPC (`core/ipc.{fish,zsh,bash}`)

**Responsibility**: JSON communication with agent over Unix socket

**Shell-agnostic protocol**:
```json
Request:  {"op":"git","cwd":"/path","format":"json"}
Response: {"branch":"main","staged":0,"unstaged":1,...}
```

**Shell-specific implementation**:
- Fish: `__gpy_request` orchestrates (`fish/core/ipc.fish`): instant cache, then `__gpy_build_data_payload` and `__gpy_ipc_send`, which uses `socat` and falls back to `nc -U`; then the oneshot fallback
- Zsh: `__gpy_request` (`zsh/core/ipc.zsh`) builds the request with `__gpy_build_data_request_into` and sends it with `__gpy_send_json`, using the `zsh/net/socket` builtin module (`zsocket`), falling back to `socat`, then `nc -U`
- Bash: `__gpy_request` (`bash/core/ipc.bash`), `__gpy_build_data_request_into` and `__gpy_send_json`, using `socat`, falling back to `nc -U`

**Common pattern**: build the request, send it, classify the reply. Fish:
```fish
set -l payload (__gpy_build_data_payload git "$root" ansi "$is_last" "$prev_bg" "$is_first")
set -l reply (__gpy_ipc_send $payload (__gpy_ipc_timeout_ms))
set -l send_status $status
# 0: complete reply; 1: unreachable; 2: sent, but no complete reply in time
#    (an empty line included); 3: refused for good, a reply starting {"error"
```

#### Layer 2: Signal Handling (Fish: `core/ipc.fish`; Zsh/Bash: `core/signals.{zsh,bash}`)

**Responsibility**: Receive the agent's SIGURG doorbell, act on any flag file, repaint the prompt

**Shell-agnostic behavior** ([ADR-0007](adr/adr-0007-sigurg-doorbell-notifications.md)): the agent sends only SIGURG, whose default disposition is ignore, so a shell without a handler (e.g. mid-`exec`) is never killed. Meaning travels in empty flag files under `<runtime_root>/shells/`, written by the agent before the signal. One handler:
1. `<pid>.reregister` exists → remove it, re-register with the agent
2. `<pid>.reload` exists → remove it, reload theme/config variables
3. Always repaint (no flag = plain repaint: git/clock update)

**Shell-specific implementation**:
- Fish: `__gpy_doorbell_handler` (`--on-signal SIGURG`, `fish/core/ipc.fish`) → increment `$__gpy_repaint_trigger`; `__gpy_repaint_on_variable` then runs `commandline -f force-repaint`
- Zsh: `TRAPURG()` → `__gpy_on_doorbell` → re-render `PROMPT`, `zle reset-prompt` when zle is active
- Bash: no trap; SIGURG stays ignored. `__gpy_precmd` handles flags 1 and 2 before rendering `PS1`, so changes show at the next prompt

#### Layer 3: Segment Rendering (`segments/*.{fish,zsh,bash}`)

**Responsibility**: Format segment output for shell's prompt syntax

**Shell-agnostic data**: the agent's rendered segment text in the shell's format (see Format Negotiation below). Only `json` replies are data objects, and those are for tooling.

**Shell-specific rendering**:
- Fish: `fish_prompt` (`fish/functions/fish_prompt.fish`) loops over `$__enabled_segments`, calling `segment_<name>_detect`, `segment_<name>_omit` and `segment_<name>_render`, which print the segment
- Zsh: `__gpy_segment_<name>` per segment (`zsh/segments/`); `__gpy_render_prompt` joins them and `__gpy_update_prompts` assigns `PROMPT` (`zsh/core/init.zsh`)
- Bash: `__gpy_segment_<name>` per segment (`bash/segments/`); `__gpy_precmd` joins them into `PS1` (`bash/core/init.bash`)

### Format Negotiation

Every data request names a `format` (`Format` in `gpy-agent/src/formatter/mod.rs`):

| Format | Used by | Reply |
|--------|---------|-------|
| `ansi` | Fish | One line with ANSI SGR escapes |
| `zsh-prompt` | Zsh | SGR escapes inside `%{ %}` |
| `bash-prompt` | Bash | SGR escapes inside `\[ \]` |
| `json` | `gpy-agent oneshot` default and tooling | A flat object per `gpy-agent/schemas/response.json` |

### Segment Rendering Boundary

The agent renders each segment, colors and separators included, into a
shell-ready string in the requested format. The shell only orders segments and
concatenates them into the prompt.

### Shell-Specific Constraints

#### Fish
- Reference implementation
- Fast string processing; no JSON parser: requests are built by hand (`__gpy_json_escape`) and replies are rendered text, printed as is apart from the `{"error"` prefix check
- Signal handling through a variable-change repaint ([ADR-0006](adr/adr-0006-async-terminal-updates.md))

#### Zsh
- `TRAPURG` plus `zle reset-prompt` repaints an idle prompt
- `PROMPT` expansion for segments

**Limitations**:
- No native JSON parsing (uses the `zsh/net/socket` builtin, `socat`, or `nc -U` for IPC)
- Prompt escaping requires `%%` for literal `%`

#### Bash
- Same segments; live updates appear at the next prompt

**Limitations**:
- No native JSON parsing (uses `socat` or `nc -U`)
- `PROMPT_COMMAND` runs synchronously on each prompt without Fish/Zsh repaint hooks
- Signal handling more complex (no clean repaint primitive)
- Code avoids associative arrays, so it runs on Bash 3.2 (macOS default)
- Duration needs `EPOCHREALTIME` (Bash 5.0+); Bash 3.2 disables that segment

See [bash-limitations.md](../user/bash-limitations.md) for details.

### Cross-Shell Compatibility Patterns

#### Pattern 1: Conditional Feature Detection

```bash
# In shell integration
if command -v socat &>/dev/null; then
    # Use socat for IPC (reliable)
elif command -v nc &>/dev/null; then
    # Fall back to nc -U (Fish/Zsh/Bash; Zsh also has the zsh/net/socket builtin)
else
    # Oneshot mode (no agent communication)
fi
```

#### Pattern 2: Shell-Agnostic Config

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

The agent renders the theme and config for each shell into `<cache dir>/theme-export.{fish,zsh,bash}` (`cache/theme_export.rs`). Shells source that file, and fall back to running `gpy-agent theme export --shell <shell>` when it is missing or lacks the version sentinel. Each line sets a shell variable such as `__color_clock_bg`, `__icon_prompt` or `__gpy_cfg_transient_prompt`:
- Fish: `set -g __color_clock_bg "black"`
- Zsh: `typeset -g __color_clock_bg="black"`
- Bash: `__color_clock_bg="black"`

The first line is the sentinel (`set -g __gpy_export_version "1"`, `typeset -g __gpy_export_version="1"`, `__gpy_export_version="1"`). A shell sources a file only when its first line matches. An older agent's export used `set -gx GPY_*`, which the shell would read back as user input, so that format is refused.

#### Pattern 3: Graceful Degradation

All shells support **oneshot mode** when agent is unavailable:
```fish
# Fish example (fish/segments/git.fish): instant-cache miss with the agent disabled
set -l fresh (__gpy_oneshot_fallback git "$root" ansi "$is_last" "$is_first" | string collect)
# runs: gpy-agent oneshot git --cwd "$root" --format ansi [--not-last] [--first]
```

**Performance impact**:
- With agent: a git or language segment with a cache file costs a file read and no IPC; IPC happens on a cold miss, a variant mismatch or a background refresh
- Without agent (oneshot): a `gpy-agent` process spawn plus the git and language work, on every prompt, at most one spawn per render

### Live Updates Across Shells

**Challenge**: Signal handling and repaint primitives differ per shell

**Solution**: Each shell installs one SIGURG handler that checks the flag files, then repaints with its own primitive

```fish
# Fish: a function declared with --on-signal SIGURG
#   handles <pid>.reregister / <pid>.reload, then:
set -g __gpy_repaint_trigger (math $__gpy_repaint_trigger + 1)
# Variable change triggers prompt repaint via hook
```

```zsh
# Zsh: TRAPURG() handles the flags, re-renders PROMPT, then
zle reset-prompt  # Zsh-specific repaint primitive
```

```bash
# Bash: no URG trap. The next prompt handles the flags and renders PS1.
# Readline cannot redraw an idle prompt; the next prompt shows fresh data.
```

**Key insight**: Agent sends the same SIGURG to all shells. Each shell handles it according to its capabilities.

### Testing Multi-Shell Compatibility

What each shell supports is the [Shell feature matrix](../INSTALL.md#shell-feature-matrix).

**Test strategy**:
1. **Unit tests**: Agent (shell-agnostic), in Rust
2. **Integration tests**: Each shell separately - IPC contract validation
3. **E2E tests**: Each shell on a real pseudo-terminal against a real agent; a clean-container fresh install

For reproducible shell-integration testing, see:
- [`tests/fish/README.md`](../../tests/fish/README.md)
- [`gpy-agent/tests/README.md`](../../gpy-agent/tests/README.md)
- [Testing Tiers](testing.md)

### Live Update Sequence

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
Terminal B: Receives SIGURG (no flag file → repaint)
      ↓
Fish SIGURG handler bumps $__gpy_repaint_trigger → commandline -f force-repaint
      ↓
Prompt redraws with fresh git status
```

**Terminal A gets two updates**:
1. Immediate: Prompt renders after command (shows stale cache)
2. Async: ~200ms later, receives SIGURG and updates to fresh status

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
│                 Fish Shell Process(es)                      │
│                                                              │
│  fish_prompt() → segments → __gpy_request → socket          │
│                                                              │
│  SIGURG handler → flags → commandline -f repaint            │
└─────────────────────────────────────────────────────────────┘
```

### Module Dependency Graph

```
error.rs (foundation)
  ↑
  ├── config/ (TOML loading, validation)
  │
  ├── theme/ (theme loading, shell export)
  │
  ├── security.rs (path validation)
  │
  ├── cache/ (bounded.rs, policy.rs, ttl_map.rs, instant_prompt/, theme_export.rs)
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
| **Theme State** | `theme::manager::ThemeManager` | `ThemeConfig` | `Arc<ThemeManager>` | • Owner: Loads themes, generates shell exports<br>• Consumers: Call `export(shell, &config, &palette)` for a shell's export, `get()` for the theme<br>• Hot-reload: Theme can change, always call methods | Allowed: `theme_mgr.export(shell, &config, &palette)` in agent<br>Allowed: `let theme = theme_mgr.get()` for a shared snapshot<br>Not allowed: Caching theme colors locally |
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

**Key Insight**: Rust ownership cleans up state automatically; only watchers and servers need an explicit stop.

---

## Key Data Flows

### Git Status Request

```
Fish segment_git_render()
  ↓
__gpy_read_instant_cache → <cache dir>/instant-prompts/<key>.git….ansi
  ↓
hit (fresh or stale): print the file; a stale hit also fires a throttled
__gpy_maybe_refresh (steps below, reply discarded). Only a miss continues.
  ↓
__gpy_build_data_payload git "$root" ansi … → __gpy_ipc_send
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
Fish: printf '%s' $synced
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
  - Notify all registered clients with SIGURG
  ↓
Fish processes receive SIGURG (no flag file)
  ↓
SIGURG handler bumps $__gpy_repaint_trigger → commandline -f repaint
  ↓
Prompt redraws (calls segment_git_render again)
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

**Key insight**: most user-visible behaviors follow one pattern: filesystem
event → watcher → agent cache update → SIGURG (plus a `<pid>.reload` flag for
config/theme) → shell repaint. `agent/mod.rs` and `agent/events/` coordinate it.

---

## IPC Protocol Specification

### Overview

GPY uses a **JSON-based request/response protocol** over Unix domain sockets. The protocol is **shell-agnostic** - any shell that can write JSON and read from a socket can communicate with the agent.

### Transport Layer

**Socket Location** (`paths::SocketPath`, `paths::runtime_root_for`):
- Override: `--socket <path>` on `gpy-agent start`/`stop`/`status`/`config reload`, or `$GPY_AGENT_SOCKET_PATH` (empty counts as unset)
- Default: `<runtime root>/gpy.sock`, where the runtime root is the first of `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy`, `$HOME/.cache/gpy` and `/tmp/gpy`; relative `XDG_*` values are ignored
- There is no `config.toml` key. Fish, Bash and Zsh resolve the same path in `__gpy_ipc_endpoint`; `gpy debug paths` prints it
- Only `gpy-agent start` creates the default runtime directory (`agent::lifecycle::ensure_runtime_dir`); other commands only resolve the path
- The path must fit the Unix socket limit (103 bytes on macOS, 107 on Linux); `gpy-agent` refuses a longer one

**Connection Type**: SOCK_STREAM (stream socket, not datagram)

**Protocol**: Line-delimited
- Each request is a single JSON object terminated by newline (`\n`)
- Each reply is one newline-terminated line. For the prompt formats (`ansi`, `zsh-prompt`, `bash-prompt`) it is the rendered segment text; for `json` requests and control ops it is a JSON object
- Multiple requests can be sent over the same connection (connection pooling)

**Refusals**: a request the agent refuses (bad JSON, unknown op, missing field, a denylisted or invalid path) gets a single `{"error":"Parse error: ..."}` line in every format, never an empty line. A shell treats any reply that starts `{"error"` as a permanent refusal: it never prints or memoizes it and falls back to oneshot (Fish: `__gpy_ipc_send` status 3). An empty line still means the agent is alive but late: the shell omits the segment and does not fork oneshot (`__gpy_ipc_send` status 2).

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
  a check of what git already knows changed. Every command runs with `core.fsmonitor` off, so a repository's config cannot make a prompt run a hook command
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
1. $GPY_CONFIG_PATH (explicit override; empty counts as unset)
2. $XDG_CONFIG_HOME/gpy/config.toml
3. ~/.config/gpy/config.toml
4. Built-in defaults (config::Config::default())
```

The first existing file wins (`config/location.rs`). A theme is not a separate file in this chain: `ui.theme` names a theme, resolved user (`~/.config/gpy/themes/<name>.toml`), then plugin, then built-in (`default`, `text`, `starship`).

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
   - Fish increments `__gpy_repaint_trigger`
   - Variable change handler calls `force-repaint`
   - No direct `commandline -f` calls from signal handlers (causes timing issues)

2. **Agent Must Send a Reload for ALL Config Changes**: When config changes affect visible segments (like removing `git` from `ui.enabled_segments`), the agent MUST send a reload notification even when disabling features. This ensures Fish reloads theme variables (`__enabled_segments`) so segments can correctly determine which is last.

3. **Theme Loading Must Use Source, Not Eval**: the agent writes the export cache file before the reload doorbell, and Fish sources it. The fallback joins the output of `gpy-agent theme export --shell fish` with newlines and pipes it to `source`:
   ```fish
   string join \n -- $export_text | source
   ```
   NOT `eval (...)` - command substitution flattens newlines, breaking multi-line scripts.

4. **Avoid Fish-Side Config Detection**: Don't use `--on-event fish_prompt` to detect config file changes - it creates race conditions with the agent's file watcher. Let the agent be the single source of truth for config changes.

**Common Pitfalls**:
- Using `commandline -f execute` for repaints → creates blank prompt lines
- Multiple signal handlers → causes double-repaints
- Forgetting the reload notification in disabled-feature paths → Fish keeps stale segment list
- Using `eval` instead of `source` → theme variables don't get set

**Test Coverage**:

Hot-reload tests:

1. **Reload Notification Delivery** (`config_hot_reload_tests.rs`):
   - Reload notification sent when disabling git
   - Reload notification sent when enabling git
   - Edge case with multiple feature toggles

2. **Theme Export Updates** (`config_hot_reload_tests.rs`, `theme_manager_tests.rs`):
   - `test_theme_export_updates_enabled_segments_when_git_disabled` - Removes git from `__enabled_segments`
   - `test_theme_export_includes_git_when_enabled` - Includes git in `__enabled_segments`
   - `test_theme_export_updates_enabled_segments_when_language_disabled` - Language toggle
   - `test_theme_export_updates_enabled_segments_when_both_disabled` - Multiple features

3. **Config Watcher** (`config_hot_reload_tests.rs`):
   - `test_config_watcher_triggers_reload_on_git_toggle` - File watcher detects changes

4. **CLI Integration** (`cli_integration_tests.rs`):
   - `test_theme_export_delimiter_selection_after_git_disabled` - Verifies delimiter selection
   - `test_config_hot_reload_git_toggle_full_scenario` - End-to-end config toggle

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
- `config_hot_reload_tests.rs`: Reload notification delivery, theme export updates, config watcher
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

An agent-rendered segment touches all three shells, each with its own `segments/` directory:
1. `fish/segments/<id>.fish`: `segment_<id>_detect`, `segment_<id>_omit`, `segment_<id>_render`
2. `zsh/segments/<id>.zsh` and `bash/segments/<id>.bash`: `__gpy_segment_<id>_detect`, `__gpy_segment_<id>_omit`, `__gpy_segment_<id>`
3. The segment must be in `ui.enabled_segments`; the shells read the resulting `__enabled_segments` from the theme export, with fallbacks in `fish/core/defaults.fish`
4. Test with both agent and oneshot modes

### Modifying IPC Protocol

**CRITICAL**: Protocol changes break compatibility

Steps:
1. Add the op to `Op::ALL` in `gpy-agent/src/ipc/wire.rs` (the only list of op names) and the `Message` enum in `gpy-agent/src/ipc/mod.rs`
2. Handle it in the `into_message` dispatch (`ipc/protocol/mod.rs`) and route it in `HandlerRegistry::route` (`ipc/handlers/mod.rs`)
3. Update the schemas in `gpy-agent/schemas/` (`message.json`, `response.json`); `gpy-agent/tests/schema_validation_tests.rs` checks them against the code
4. Update the IPC code of all three shells: `fish/core/ipc.fish`, `zsh/core/ipc.zsh`, `bash/core/ipc.bash`
5. Bump `PROTOCOL_VERSION` if the change is incompatible (shells compare it for equality; there is no negotiation)
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
