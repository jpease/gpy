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

**Fish Shell** (`segments/`, `core/`, `functions/`):
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

```
1. segment_git_render → __gpy_request git $PWD
     Fish:     __gpy_build_data_payload … ansi → __gpy_ipc_send
     Zsh/Bash: __gpy_build_data_request_into … zsh-prompt|bash-prompt → __gpy_send_json
2. {"op":"git","cwd":…,"format":"ansi"}\n over the Unix socket → gpy-agent
3. agent reuses a fresh cache entry, or runs git status --porcelain=v2
4. agent renders with the theme in the requested format
5. ← one line of rendered text, which the shell prints
```

**Timing**: a cached response costs one IPC round trip (`ipc_ping` baseline 0.037 ms); a fresh git query on a 100-file repository baselines at 17.2 ms (`git_status_100_files`). Both figures are from `tests/performance-baselines.json`.

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
- Fish: `__gpy_request()` using `socat`, falling back to `nc -U`
- Zsh: `__gpy_request()` using the `zsh/net/socket` builtin module (`zsocket`), falling back to `socat`, then `nc -U`
- Bash: `__gpy_request()` using `socat`, falling back to `nc -U`

**Common pattern**: Request format + IPC transport
```fish
# Fish example
function __gpy_request -a operation cwd format is_last
    set -l request (string escape --json -- "$operation" "$cwd" "$format" "$is_last")
    echo "{\"op\":\"$operation\",\"cwd\":$request[2],\"format\":$request[3]}" | socat - UNIX-CONNECT:$socket
end
```

#### Layer 2: Signal Handling (`core/signals.{fish,zsh,bash}`)

**Responsibility**: Receive the agent's SIGURG doorbell, act on any flag file, repaint the prompt

**Shell-agnostic behavior** ([ADR-0007](adr/adr-0007-sigurg-doorbell-notifications.md)): the agent sends only SIGURG, whose default disposition is ignore, so a shell without a handler (e.g. mid-`exec`) is never killed. Meaning travels in empty flag files under `<runtime_root>/shells/`, written by the agent before the signal. One handler:
1. `<pid>.reregister` exists → remove it, re-register with the agent
2. `<pid>.reload` exists → remove it, reload theme/config variables
3. Always repaint (no flag = plain repaint: git/clock update)

**Shell-specific implementation**:
- Fish: one `--on-signal SIGURG` function → increment `$__gpy_repaint_trigger`
- Zsh: `TRAPURG()` → re-render `PROMPT`, `zle reset-prompt` when zle is active
- Bash: no trap; SIGURG stays ignored. `__gpy_precmd` handles flags 1 and 2 before rendering `PS1`, so changes show at the next prompt

#### Layer 3: Segment Rendering (`segments/*.{fish,zsh,bash}`)

**Responsibility**: Format segment output for shell's prompt syntax

**Shell-agnostic data**: Same JSON from agent
```json
{"branch":"main","staged":2,"unstaged":1,"state":"dirty"}
```

**Shell-specific rendering**:
- Fish: `set -g fish_prompt_segments (segment_git)`
- Zsh: `PROMPT='$(segment_git)$(segment_dir)'`
- Bash: `PS1="$(segment_git)$(segment_dir)"`

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
- Fast string processing, native JSON parsing
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
- `PROMPT_COMMAND` less elegant than Fish/Zsh hooks
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
  └── theme.toml     # Shared by all shells
```

Agent exports theme to shell-specific variables:
- Fish: `set -gx GPY_COLOR_GIT_CLEAN "#00ff00"`
- Zsh: `export GPY_COLOR_GIT_CLEAN='#00ff00'`
- Bash: `export GPY_COLOR_GIT_CLEAN='#00ff00'`

#### Pattern 3: Graceful Degradation

All shells support **oneshot mode** when agent is unavailable:
```fish
# Fish example
if not __gpy_agent_available
    # Fall back to direct git commands
    set -l branch (git branch --show-current 2>/dev/null)
    # Slower, but works without agent
end
```

**Performance impact**:
- With agent: one IPC round trip per segment on a cache hit
- Without agent (oneshot): a `gpy-agent` process spawn plus the git and language work, on every prompt

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
  ├── config/ (TOML loading)
  │
  ├── security/ (path validation)
  │
  ├── git/
  │   ├── cache.rs (Git status caching)
  │   ├── status.rs (backend-agnostic repository state)
  │   └── native/
  │       ├── mod.rs (git subprocess backend)
  │       └── parser.rs (porcelain v2 parsing)
  │
  ├── language/
  │   ├── cache.rs (language detection caching)
  │   ├── detector.rs (gengo-language matchers + owned ignore walk)
  │   ├── filters.rs (linguist vendor/doc exclusion globs)
  │   └── version.rs (version detection trait)
  │
  ├── ipc/
  │   ├── protocol.rs (JSON serialization)
  │   ├── server.rs (request dispatcher)
  │   └── registry.rs (Fish PID tracking)
  │
  └── watcher/
      ├── filesystem.rs (notify integration)
      ├── debouncer.rs (event coalescing)
      └── multi_repo.rs (multi-repo coordination)

agent/mod.rs (orchestrator - coordinates above)

main.rs (CLI entry point)
```

**Key insight**: Clean, acyclic dependency graph. Each layer depends only on lower layers.

---

## Threading Model

### Agent runtime

- tokio multi-thread runtime (`Runtime::new()`, `agent/lifecycle/start.rs`).
- `Agent::start_background` (`agent/mod.rs`) runs one 5-branch `tokio::select!`:
  `server_task` (the spawned IPC server stopped), `shutdown_signal`
  (SIGINT/SIGTERM), `clock_timer` (1 s, gated on `agent.live_updates`),
  `pruning_timer` (60 s: prune dead clients, re-nudge shells) and
  `reconcile_timer` (45 s `RECONCILE_INTERVAL_SECS`,
  `GPY_RECONCILE_INTERVAL_SECS` override, gated on `live_updates`).
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
| **Git Status Cache** | `git::cache::GitStatusCache` | `HashMap<PathBuf, CacheEntry>` | `Arc<GitStatusCache>` | • Owner: Provides `get()`, `set()`, `invalidate()` methods<br>• Consumers: Read via `get()`, never direct mutation<br>• Thread-safe: Internal `Mutex` (all access serialized) | Allowed: `cache.get(&repo)` in IPC handler<br>Allowed: `cache.set(&repo, status)` in agent<br>Not allowed: Accessing internal HashMap directly |
| **Client Registry** | `ipc::registry::ClientDirectory` | `HashMap<u32, ClientInfo>` | `Arc<ClientDirectory>` | • Owner: Provides `register()`, `unregister()`, `notify_*()` methods<br>• Consumers: Register clients, send signals<br>• Thread-safe: Internal `Mutex` (all access serialized) | Allowed: `registry.register(pid, cwd)` in IPC<br>Allowed: `registry.notify_repaint()` in agent<br>Not allowed: Direct PID manipulation |
| **File Watcher** | `watcher::multi_repo::MultiRepoWatcher` | `notify::RecommendedWatcher` | `Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>` | • Owner: Agent module creates and destroys<br>• Consumers: IPC server registers repos via shared slot<br>• Lazy initialization pattern | Allowed: `watcher.register(repo)` via agent<br>Allowed: Agent creates watcher in `Agent::new()`<br>Not allowed: Creating watcher outside agent module |
| **Configuration** | `config::manager::ConfigManager` | `Config` struct | `Arc<ConfigManager>` | • Owner: Provides `get_current()`, `reload()` methods<br>• Consumers: Read config, never cache locally<br>• Hot-reload: Config can change, always use fresh copy | Allowed: `config_mgr.get_current()` in handlers<br>Allowed: `config_mgr.reload()` on file events<br>Not allowed: Storing `Config` directly (stale data risk) |
| **Theme State** | `theme::manager::ThemeManager` | `ThemeConfig` | `Arc<ThemeManager>` | • Owner: Loads themes, generates Fish exports<br>• Consumers: Call `export_fish()` for Fish vars, `get()` to clone config<br>• Hot-reload: Theme can change, always call methods | Allowed: `theme_mgr.export_fish(&config)` in agent<br>Allowed: `let theme = theme_mgr.get()` to clone<br>Not allowed: Caching theme colors locally |
| **Language Version Cache** | `language::version` module | Global `VERSION_CACHE` in `gpy-agent/src/language/version.rs` | `TtlMap<String, String>` (capped at `VERSION_CACHE_CAPACITY`): successful probes only; failures go to `VERSION_FAILURE_CACHE` | • Owner: `language::version` module manages global cache<br>• Consumers: Version detectors use implicitly via `detect_language_release()`<br>• TTL: 24 hour expiration (configurable)<br>• Thread-safe: `TtlMap` is internally synchronized | Allowed: Call `detect_language_release()`, cache automatic<br>Allowed: Cache shared globally, TTL handles staleness<br>Not allowed: Creating separate per-request version caches |
| **IPC Server State** | `ipc::server::EndpointHandle` | Socket listener | Owned by `Agent` | • Owner: Agent creates and holds server<br>• Lifecycle: Lives until agent shutdown<br>• Thread-safe: Tokio runtime handles connections | Allowed: Agent creates in `Agent::new()`<br>Allowed: Server runs in `Agent::start_background()`<br>Not allowed: Multiple server instances |

#### State Access Patterns

**Correct Pattern**: Access state via owner's public API
```rust
// CORRECT: Use owner's API
let status = git_cache.get(&repo_path);  // GitStatusCache::get()
clients.notify_repaint(None);            // ClientDirectory::notify_repaint()

// CORRECT: Agent coordinates state updates
impl Agent {
    fn handle_file_event(cache: &GitStatusCache, event: &Event) {
        cache.invalidate(&event.repo);    // Coordinate invalidation
        let status = load_repository_state(&event.repo);
        cache.set(&event.repo, status);   // Coordinate refresh
    }
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
| Storing `Config` directly | Misses hot-reload updates | Use `Arc<ConfigManager>`, call `get_current()` |
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
│  Agent::run() - Runtime Phase                        │
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

**Key Insight**: Rust's ownership system ensures state cleanup automatically.
No manual cleanup needed beyond stopping watchers and servers.

---

## Key Data Flows

### Git Status Request

```
Fish segment_git_render()
  ↓
__gpy_request git $PWD
  ↓
JSON: {"op":"git","cwd":"/path","format":"ansi"}
  ↓
Unix socket → gpy-agent IPC server
  ↓
Message::RepositoryStatus parsed
  ↓
security::validate_path(path) → reject if suspicious
  ↓
git_cache.get(repo_path) → check freshness
  ↓
if stale:
    git::status::load_repository_state()
      ↓
    native backend: git status --porcelain=v2 --branch
      ↓
    Parse output, build RepositoryStatus
      ↓
    git_cache.insert(repo_path, status, timestamp)
  ↓
Render response in ansi format:
  - Apply theme colors
  - Generate ANSI escape codes
  - Add delimiters (if not is_last)
  ↓
← ansi: "\x1b[32m main \x1b[0m  2 \x1b[33m⚡1\x1b[0m"
  ↓
Fish: printf '%s' $response
```

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
  - If exists: reset expiry to now + 150ms, and merge Git paths into the
    pending event (bounded; past GitPaths::MAX_PATHS it degrades to a
    whole-repo full scan)
  - If new: create entry with expiry
  ↓
(150ms passes)
  ↓
Flush thread wakes, finds expired event
  ↓
DebouncedEvent → callback
  ↓
agent::handle_file_event():
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
1 s tick (clock_timer, while agent.live_updates is on)
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

**Socket Location** (`paths::resolve_socket_path`, `paths::runtime_root_for`):
- Override: `--socket <path>` on `gpy-agent start`/`stop`/`status`, or `$GPY_AGENT_SOCKET_PATH` (empty counts as unset)
- Default: `<runtime root>/gpy.sock`, where the runtime root is the first of `$XDG_RUNTIME_DIR/gpy`, `$XDG_CACHE_HOME/gpy`, `$HOME/.cache/gpy` and `/tmp/gpy`; relative `XDG_*` values are ignored
- There is no `config.toml` key. Fish, Bash and Zsh resolve the same path in `__gpy_ipc_endpoint`; `gpy debug paths` prints it
- Only `gpy-agent start` creates the default runtime directory (`agent::lifecycle::ensure_runtime_dir`); other commands only resolve the path
- The path must fit the Unix socket limit (103 bytes on macOS, 107 on Linux); `gpy-agent` refuses a longer one

**Connection Type**: SOCK_STREAM (stream socket, not datagram)

**Protocol**: Line-delimited JSON
- Each request is a single JSON object terminated by newline (`\n`)
- Each response is a single JSON object terminated by newline
- Multiple requests can be sent over the same connection (connection pooling)

**Security**:
- The socket is chmod'ed to `0600` right after bind
- The socket's directory is trusted only if it is a real directory (not a symlink) owned by the user. An owned group- or world-writable directory is set to `0700` when it is the default runtime root and refused when it comes from `--socket` or `$GPY_AGENT_SOCKET_PATH`. `gpy-agent start` checks before it touches the directory, and the daemon checks again immediately before bind (`secure_socket_dir` in `ipc/server/socket_dir.rs`)
- No handshake or peer-credential check: any process running as the user can connect. See [SECURITY.md](../../SECURITY.md#trust-boundaries) for the full trust policy

### Operations

Each request is one newline-terminated JSON object with an `op`:

`git` `lang` `directory` `clock` `duration` `character` `hostname` `username`
`ping` `status` `shutdown` `config_reload` `register` `unregister` `workspace`

Per-op fields, `format` values and reply shapes:
[`gpy-agent/schemas/README.md`](../../gpy-agent/schemas/README.md).

### Protocol Versioning

`PROTOCOL_VERSION = 2` (`gpy-agent/src/ipc/protocol/mod.rs`); shells compare
for equality and restart the agent on mismatch. There is no negotiation.

### Performance Characteristics

A cached git status costs one IPC round trip (`tests/performance-baselines.json`: `ipc_latency` budget 1 ms target, 1.5 ms max; `ipc_ping` baseline 0.037 ms). A fresh query adds a `git status --porcelain=v2` subprocess; the per-repo-size figures live in the module docs at the top of `gpy-agent/src/git/native/mod.rs`. A fresh cache entry returns without spawning git at all.

### Security Considerations

See [SECURITY.md](../../SECURITY.md#trust-boundaries) for the trust boundaries, threat model and defenses.

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
- Inherits git's own optimizations: `core.fsmonitor` and `core.untrackedCache`
  reduce a full scan to a check of what git already knows changed
- One subprocess returns branch, ahead/behind and file status together
- `--porcelain=v2` is machine-readable and has been stable since Git 2.11.0 (2016)
- Spawn overhead is real but small against what the untracked cache saves:
  see `git_status_cold`/`git_status_warm` in `tests/performance-baselines.json`

### Why Separate Debouncing and Cache Cooldown?

**Debouncing** (100ms): Prevents redundant signals to Fish processes
- Problem solved: `git commit` writes 5 files → only 1 signal after quiet period

**Cache Cooldown** (150ms): Prevents redundant git queries
- Problem solved: File event fires before IPC request completes → skip re-query

**Both needed**: Different problems, different solutions.

### Why tokio Instead of Threads?

**Async needed for**:
- IPC server (many concurrent connections)
- Timers (clock updates)
- Signal handling (shutdown)

**Threads used for**:
- File watcher flush (periodic, not event-driven)

---

## Security Model

See [SECURITY.md](../../SECURITY.md#trust-boundaries) for the trust boundaries, threat model and defenses.

---

## Performance Characteristics

### Latency Targets

| Operation | Target | Baseline | Notes |
|-----------|--------|---------|-------|
| IPC round-trip | 1 ms (max 1.5 ms) | 0.037 ms (`ipc_ping`) | Cached response |
| Git status (warm) | 50 ms (max 100 ms) | 17.2 ms (`git_status_100_files`) | Depends on repo size |
| Git status (cold) | 500 ms (max 750 ms) | — | First query in a repository |
| Prompt render (cached) | 20 ms (max 50 ms) | — | All segments |

### Memory Footprint

- Agent process: 2-5 MB resident
- Cache per repo: ~200 bytes (metadata only)
- Client registry: ~50 bytes per Fish process

### Caching Strategy

**Git status cache**:
- TTL: Determined by cooldown window (150ms default)
- Eviction: Explicit invalidation on file events
- Size: Unbounded (assumes reasonable number of repos)

**Language detection cache**:
- TTL: Configurable via config (default: 5 minutes)
- Eviction: LRU (not yet implemented, currently no eviction)
- Size: Unbounded

---

## Configuration System

### Loading Hierarchy

```
1. $XDG_CONFIG_HOME/gpy/config.toml (user config)
2. $XDG_CONFIG_HOME/gpy/theme.toml (user theme)
3. Built-in defaults (config::Config::default())
```

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

**Currently supported** (no restart required):
- Theme changes (colors, glyphs, segment styling)
- Config changes (git.enabled, language.enabled, segment enabling/disabling)
- Git status updates (cross-terminal via file watcher)
- Clock updates (every minute)

**Not supported**: Git timeout, language cache TTL (require agent restart)

### Hot-Reload Implementation

**Critical Design Decisions**:

1. **Unified Signal Path**: Repaints (git/clock) and reloads (config/theme) arrive as the same SIGURG; a reload is a SIGURG preceded by a `<pid>.reload` flag file:
   - Agent sends signal
   - Fish increments `__gpy_repaint_trigger`
   - Variable change handler calls `force-repaint`
   - No direct `commandline -f` calls from signal handlers (causes timing issues)

2. **Agent Must Send a Reload for ALL Config Changes**: When config changes affect visible segments (like `git.enabled = false`), the agent MUST send a reload notification even when disabling features. This ensures Fish reloads theme variables (`GPY_GIT_ENABLED`, `__enabled_segments`) so segments can correctly determine which is last.

3. **Theme Loading Must Use Pipe-to-Source**:
   ```fish
   gpy-agent theme export --format fish | source
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

These tests specifically cover the critical bug where disabling `git.enabled` would not send a reload notification, causing Fish to use stale segment lists and incorrect delimiters.

---

## Testing Strategy

### Test Pyramid

```
        ╱╲
       ╱E2E╲       E2E: agent_daemon_tests.rs, e2e_ipc_tests.rs
      ╱────╲
     ╱ Integ╲      Integration: language_tests.rs, git_status_tests.rs
    ╱────────╲
   ╱   Unit   ╲    Unit: watcher/tests, config/tests, security/tests
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
- `e2e_agent_autostart.test.fish`: Auto-start on Fish init
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

# Fish integration tests
./scripts/test_fish.sh

# Interactive sessions on a real pseudo-terminal (idle repaint, two shells,
# a config reload reaching a Fish client)
fish tests/fish/e2e_interactive_session.test.fish
```

---

## Common Operations

### Adding a New Segment

See [add-new-segment.md](add-new-segment.md) for the checklist.

Quick version:
1. Create `segments/my_segment.fish`
2. Implement `segment_my_segment_detect()` and `segment_my_segment_render()`
3. Add to `__enabled_segments` in `functions/fish_prompt.fish`
4. Test with both agent and oneshot modes

### Modifying IPC Protocol

**CRITICAL**: Protocol changes break compatibility

Steps:
1. Update `Message` enum in `ipc/mod.rs`
2. Update Fish IPC code in `core/ipc.fish`
3. Update protocol version negotiation notes if compatibility requirements change
4. Document the change for shell integrations
5. Coordinate Fish + Rust deployment

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
4. **Run tests**: `cargo test`, `./scripts/test_fish.sh`
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
- Add config hot-reload support
- Implement protocol versioning
- Performance optimization

---

## Further Reading

- [add-new-segment.md](add-new-segment.md) - How to add segments
- [ADR-0004: SIGUSR1 Live Updates](adr/adr-0004-sigusr1-live-updates.md) - Why the watcher signals shells instead of polling
- [ADR-0007: SIGURG Doorbell](adr/adr-0007-sigurg-doorbell-notifications.md) - The current single-signal protocol and flag files
- [CONTRIBUTING.md § Quality Gates](../../CONTRIBUTING.md#quality-gates) - How to run every suite

---

**Questions?** Open an issue with the `question` label or ask in discussions.
