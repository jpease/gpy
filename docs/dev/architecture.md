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
User types command → Fish renders prompt → segment_git_render()
                                              ↓
                        __gpy_request "git" $PWD "fish-ansi" false
                                              ↓
                        JSON over Unix socket → gpy-agent
                                                     ↓
                              git status --porcelain=v2 subprocess
                                                     ↓
                                      Check cache (fresh? return cached)
                                                     ↓
                                      Render ANSI codes with theme
                                                     ↓
                        ← fish-ansi response ← gpy-agent
                                              ↓
                        printf '%s' $response
```

**Typical timing**: <0.1ms for cached response, ~15-20ms for fresh git query

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
- Bash: `trap '...' URG` → re-render `PS1` (visible at the next prompt)

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

The agent supports multiple output formats via the `format` field:

```
┌─────────────────────────────────────────────────────┐
│              Format Types (Request)                 │
├─────────────────────────────────────────────────────┤
│  "json"           → Raw data for shell processing   │
│  "fish-ansi"      → ANSI codes for Fish display     │
│  "zsh-ansi"       → ANSI codes for Zsh display      │
│  "bash-ansi"      → ANSI codes for Bash display     │
│  "fish-rendered"  → Fully rendered Fish prompt      │
└─────────────────────────────────────────────────────┘
```

**Current status**:
- ✅ `json`: Fully supported (all shells use this)
- ✅ `fish-ansi`: Legacy format (deprecated but supported)
- ⏳ `*-rendered`: Planned for zero-latency instant prompts

**Design decision**: Start with `json` format to keep shell integrations flexible. Future optimization: pre-rendered formats to eliminate shell-side formatting.

### Segment Rendering Boundary

**Where does rendering happen?**

```
Agent Side (Rust)              Shell Side (Fish/Zsh/Bash)
─────────────────             ──────────────────────────

Git status query    ───→
Language detection  ───→
Theme data          ───→
                                            ←─── Segment composition
                                            ←─── ANSI color codes
                                            ←─── Delimiter placement
                                            ←─── Prompt string assembly
```

**Current design** (as of 2025-12-22):
1. **Agent provides**: Raw data (branch name, file counts) + theme colors
2. **Shell assembles**: ANSI codes, delimiters, prompt string
3. **Boundary**: JSON response from IPC

**Why this boundary?**
- **Flexibility**: Shells can customize rendering (e.g., Bash limitations)
- **Simplicity**: Agent doesn't need prompt syntax knowledge
- **Performance**: Rendered segments can be cached in shell variables

**Future optimization**:
- Agent pre-renders ANSI strings → zero shell-side processing
- Requires format="fish-rendered" support
- Trade-off: Less flexible, but faster prompts

### Shell-Specific Constraints

#### Fish
- ✅ Full feature support (reference implementation)
- ✅ Fast string processing, native JSON parsing
- ✅ Clean signal handling via `fish_prompt`

#### Zsh
- ✅ Full feature support
- ✅ Async hooks (zle) for signal handling
- ✅ `PROMPT` expansion for segments

**Limitations**:
- No native JSON parsing (uses the `zsh/net/socket` builtin, `socat`, or `nc -U` for IPC)
- Prompt escaping requires `%%` for literal `%`

#### Bash
- ✅ Core features supported
- ⚠️ Limited compared to Fish/Zsh

**Limitations**:
- No native JSON parsing (uses `socat` or `nc -U`)
- `PROMPT_COMMAND` less elegant than Fish/Zsh hooks
- Signal handling more complex (no clean repaint primitive)
- Associative arrays require Bash 4.0+
- macOS ships with Bash 3.2 (2007) - users should upgrade to 5.0+

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
- With agent: <0.1ms per prompt (cached IPC)
- Without agent (oneshot): 50-200ms per prompt (spawns git processes)

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
# Bash: trap '...' URG handles the flags, re-renders PS1.
# Readline cannot redraw an idle prompt; the next prompt shows fresh data.
```

**Key insight**: Agent sends the same SIGURG to all shells. Each shell handles it according to its capabilities.

### Testing Multi-Shell Compatibility

**Test matrix**:
```
           Fish 3.6+  │  Zsh 5.8+  │  Bash 4.0+  │  Bash 5.0+
─────────────────────┼────────────┼─────────────┼────────────
IPC (json)       ✅  │     ✅     │      ✅     │     ✅
Live updates     ✅  │     ✅     │      ⚠️     │     ✅
Config reload    ✅  │     ✅     │      ✅     │     ✅
Theme reload     ✅  │     ✅     │      ✅     │     ✅
Oneshot mode     ✅  │     ✅     │      ✅     │     ✅
```

**Test strategy**:
1. **Unit tests**: Agent (shell-agnostic) - 600+ tests in Rust
2. **Integration tests**: Each shell separately - IPC contract validation
3. **E2E tests**: Docker containers for Fish/Zsh/Bash environments

For reproducible shell-integration testing, see:
- [`tests/fish/README.md`](../../tests/fish/README.md)
- [`gpy-agent/tests/README.md`](../../gpy-agent/tests/README.md)

### Live Updates (The Magic)

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
Fish SIGURG handler bumps $__gpy_repaint_trigger → commandline -f repaint
      ↓
Prompt redraws with fresh git status (✨ magic!)
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
  │   ├── transport.rs (socket I/O)
  │   ├── server.rs (request dispatcher)
  │   ├── client.rs (client utilities)
  │   └── registry.rs (Fish PID tracking)
  │
  └── watcher/
      ├── filesystem.rs (notify integration)
      ├── debouncer.rs (event coalescing)
      └── multi_repo.rs (multi-repo coordination)

agent.rs (orchestrator - coordinates above)

main.rs (CLI entry point)
```

**Key insight**: Clean, acyclic dependency graph. Each layer depends only on lower layers.

---

## Threading Model

### Agent Event Loop (async, single-threaded)

```rust
tokio::select! {
    // IPC server accepts connections
    result = server_future => { /* handle server completion */ }

    // Shutdown signal (SIGTERM/SIGINT)
    shutdown = shutdown_signal => { /* graceful cleanup */ }

    // Clock timer (1s or 60s intervals)
    _ = clock_timer.tick() => {
        // Send SIGURG (repaint) to all registered shells
    }
}
```

**Why tokio::select?**
- Allows immediate response to shutdown signals
- Coordinates IPC server + clock timer concurrently
- All I/O is async (no blocking)

### Watcher Background Thread

The file watcher uses **one dedicated OS thread** for debounce flushing:

```
Main Thread                     Flush Thread
     │                               │
     ├─ notify event                 │
     ├─ filter event                 │
     ├─ feed debouncer               │
     │                               │
     │                          sleep 150ms
     │                               │
     │                          acquire lock
     │                          flush expired
     │                          release lock
     │                               │
     ├─ another event                │
     ├─ reset expiry                 │
     │                          sleep 150ms
     │                               │
```

**Why a dedicated thread?**
- Debouncer needs periodic flushing independent of events
- If events stop arriving, pending events would never expire
- sleep-based thread is lightweight (minimal CPU)

### Shared State (Thread-Safe via Arc)

```rust
Arc<GitStatusCache>       // Shared: agent + IPC server
Arc<ClientDirectory>      // Shared: agent + watcher + clock timer
Arc<MultiRepoWatcher>     // Shared: agent + IPC server (for registration)
```

All shared state uses interior mutability (`Mutex` or atomic types).

### State Ownership Map (Critical for AI Agents)

This table documents which modules own which pieces of state and how state is shared.
**RULE**: Only the owning module should mutate state. Other modules access via shared references.

| State | Owner Module | Type | Sharing Pattern | Access Rules | Examples |
|-------|-------------|------|-----------------|--------------|----------|
| **Git Status Cache** | `git::cache::GitStatusCache` | `HashMap<PathBuf, CacheEntry>` | `Arc<GitStatusCache>` | • Owner: Provides `get()`, `set()`, `invalidate()` methods<br>• Consumers: Read via `get()`, never direct mutation<br>• Thread-safe: Internal `Mutex` (all access serialized) | ✅ `cache.get(&repo)` in IPC handler<br>✅ `cache.set(&repo, status)` in agent<br>❌ Accessing internal HashMap directly |
| **Client Registry** | `ipc::registry::ClientDirectory` | `HashMap<u32, ClientInfo>` | `Arc<ClientDirectory>` | • Owner: Provides `register()`, `unregister()`, `notify_*()` methods<br>• Consumers: Register clients, send signals<br>• Thread-safe: Internal `Mutex` (all access serialized) | ✅ `registry.register(pid, cwd)` in IPC<br>✅ `registry.notify_repaint()` in agent<br>❌ Direct PID manipulation |
| **File Watcher** | `watcher::multi_repo::MultiRepoWatcher` | `notify::RecommendedWatcher` | `Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>` | • Owner: Agent module creates and destroys<br>• Consumers: IPC server registers repos via shared slot<br>• Lazy initialization pattern | ✅ `watcher.register(repo)` via agent<br>✅ Agent creates watcher in `Agent::new()`<br>❌ Creating watcher outside agent module |
| **Configuration** | `config::manager::ConfigManager` | `Config` struct | `Arc<ConfigManager>` | • Owner: Provides `get_current()`, `reload()` methods<br>• Consumers: Read config, never cache locally<br>• Hot-reload: Config can change, always use fresh copy | ✅ `config_mgr.get_current()` in handlers<br>✅ `config_mgr.reload()` on file events<br>❌ Storing `Config` directly (stale data risk) |
| **Theme State** | `theme::manager::ThemeManager` | `ThemeConfig` | `Arc<ThemeManager>` | • Owner: Loads themes, generates Fish exports<br>• Consumers: Call `export_fish()` for Fish vars, `get()` to clone config<br>• Hot-reload: Theme can change, always call methods | ✅ `theme_mgr.export_fish(&config)` in agent<br>✅ `let theme = theme_mgr.get()` to clone<br>❌ Caching theme colors locally |
| **Language Version Cache** | `language::version` module | Global `VERSION_CACHE` at `gpy-agent/src/language/version.rs:316` | `OnceLock<Mutex<HashMap<String, CachedVersion>>>` - global static shared across all requests | • Owner: `language::version` module manages global cache<br>• Consumers: Version detectors use implicitly via `detect_language_release()`<br>• TTL: 24 hour expiration (configurable)<br>• Thread-safe: Global `Mutex` with `OnceLock` initialization | ✅ Call `detect_language_release()`, cache automatic<br>✅ Cache shared globally, TTL handles staleness<br>❌ Creating separate per-request version caches |
| **IPC Server State** | `ipc::server::EndpointHandle` | Socket listener | Owned by `Agent` | • Owner: Agent creates and holds server<br>• Lifecycle: Lives until agent shutdown<br>• Thread-safe: Tokio runtime handles connections | ✅ Agent creates in `Agent::new()`<br>✅ Server runs in `Agent::start_background()`<br>❌ Multiple server instances |

#### State Access Patterns

**Correct Pattern**: Access state via owner's public API
```rust
// ✅ CORRECT: Use owner's API
let status = git_cache.get(&repo_path);  // GitStatusCache::get()
clients.notify_repaint(None);            // ClientDirectory::notify_repaint()

// ✅ CORRECT: Agent coordinates state updates
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
// ❌ WRONG: Accessing internal state directly
let internal_map = cache.inner.lock().unwrap();  // Breaks encapsulation

// ❌ WRONG: Duplicating state in another module
pub struct MyModule {
    cached_config: Config,  // Stale data! Use Arc<ConfigManager> instead
}

// ❌ WRONG: Cross-module coordination outside agent
impl GitCache {
    fn refresh(&self, repo: &Path, clients: &ClientDirectory) {
        // ...
        clients.notify_repaint(None);  // Coordination belongs in agent module!
    }
}
```

#### Arc Sharing Guidelines for AI Agents

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
__gpy_request "git" $PWD "fish-ansi" false
  ↓
JSON: {"op":"git","cwd":"/path","format":"fish-ansi"}
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
Render response in fish-ansi format:
  - Apply theme colors
  - Generate ANSI escape codes
  - Add delimiters (if not is_last)
  ↓
← fish-ansi: "\x1b[32m main \x1b[0m  2 \x1b[33m⚡1\x1b[0m"
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
Agent starts with show_seconds = false
  ↓
create_clock_timer():
  - Calculate seconds until next minute
  - First tick: align to 14:33:00 (wait 13s if started at 14:32:47)
  - Subsequent ticks: exactly 60s apart
  ↓
Timer tick event
  ↓
Check: should_send_clock_signal()
  - If on minute boundary: send
  - Otherwise: skip
  ↓
client_registry.notify_repaint(None)
  ↓
All shell processes receive SIGURG
  ↓
Prompt repaints with updated time
```

---

## Common Behaviors → Code Paths

This section maps user-visible behaviors to their implementation paths, helping both humans and LLMs understand "what you see is what it does."

**1. Git commit → prompt updates in ~200ms**
```
User runs: git commit -m "message"
  ↓
.git/HEAD modified
  ↓
watcher/filesystem.rs:125 - notify event filtered (accept .git/HEAD)
  ↓
watcher/debouncer.rs:87 - debounce 100ms
  ↓
agent.rs:1065 - handle_file_event() invalidates cache
  ↓
agent.rs:1102 - notify_clients_for_repo() sends SIGURG
  ↓
All Fish processes repaint with fresh git status
  ↓
Tests: gpy-agent/tests/git_status_tests.rs:test_git_status_with_staged_files
```

**2. Config change → theme reloads automatically**
```
User edits: ~/.config/gpy/config.toml
  ↓
watcher/filesystem.rs:329 - config file event detected
  ↓
agent.rs:1150 - handle_config_change() calls config_mgr.reload()
  ↓
agent writes <pid>.reload for each client, then sends SIGURG (notify_reload)
  ↓
Fish SIGURG handler sees the reload flag → sources fresh theme export
  ↓
Tests: gpy-agent/tests/config_hot_reload_tests.rs:test_config_watcher_triggers_reload
       gpy-agent/tests/cli_integration_tests.rs:test_config_hot_reload_git_toggle
```

**3. Directory change → language detection updates**
```
Fish: cd /path/to/rust/project
  ↓
Fish prompt renders → segment_language_render()
  ↓
core/ipc.fish:__gpy_request "language" $PWD
  ↓
ipc/server.rs:250 - handles Operation::Language
  ↓
language/detector.rs - gengo-language detection (cached)
  ↓
Returns: {"language":"Rust","version":"1.90.0"}
  ↓
Fish renders: " 1.90.0 "
  ↓
Tests: gpy-agent/tests/language_tests.rs:test_version_parsing_rust
       gpy-agent/tests/language_detection_integration_tests.rs
```

**4. Clock tick → time updates across all terminals**
```
Agent clock timer (60s interval if show_seconds=false)
  ↓
agent.rs:200 - create_clock_timer() ticks
  ↓
agent.rs:850 - should_send_clock_signal() checks minute boundary
  ↓
agent.rs:870 - client_registry.notify_repaint(None) sends to all clients
  ↓
All Fish terminals repaint with updated time
  ↓
Tests: gpy-agent/tests/clock_timer_tests.rs:test_timer_alignment_to_minute_boundary
```

**5. Prompt render latency budget**
```
User presses Enter → Fish renders new prompt
  ↓
Target: < 50ms total, typical: 10-20ms
  ↓
Breakdown (cached paths):
  - segment_directory: ~2ms (pwd lookup)
  - segment_git: ~0.1ms (IPC + cache hit)
  - segment_language: ~0.1ms (IPC + cache hit)
  - segment_duration: <1ms (Fish variable)
  - segment_status: <1ms (Fish variable)
  - ANSI rendering: ~3ms (color codes)
  ↓
Performance validation: time fish_prompt
```

**Key insight**: Most user-visible behaviors involve the pattern: filesystem event → watcher → agent cache update → SIGURG (plus a `<pid>.reload` flag for config/theme) → Fish repaint. The agent orchestrates this coordination in `agent.rs`, ensuring prompt updates stay fast via caching while remaining accurate via file watching.

---

## IPC Protocol Specification

### Overview

GPY uses a **JSON-based request/response protocol** over Unix domain sockets. The protocol is **shell-agnostic** - any shell that can write JSON and read from a socket can communicate with the agent.

### Transport Layer

**Socket Location**:
- Default: `$XDG_RUNTIME_DIR/gpy-agent.sock` or `/tmp/gpy-agent-$UID.sock`
- Configurable via: `$GPY_AGENT_SOCKET_PATH` or `config.toml`

**Connection Type**: SOCK_STREAM (stream socket, not datagram)

**Protocol**: Line-delimited JSON
- Each request is a single JSON object terminated by newline (`\n`)
- Each response is a single JSON object terminated by newline
- Multiple requests can be sent over the same connection (connection pooling)

**Security**:
- Socket permissions: `0600` (owner read/write only)
- OS enforces UID matching (only same user can connect)
- No authentication needed (filesystem permissions sufficient)

### Message Format

#### Request Message

All requests share this structure:

```json
{
  "op": "<operation>",
  "cwd": "<current-working-directory>",
  "format": "<response-format>",
  "is_last": <boolean>
}
```

**Fields**:
- `op` (string, required): Operation type (`"git"`, `"language"`, `"ping"`, `"register"`)
- `cwd` (string, optional): Current working directory (required for git/language operations)
- `format` (string, optional): Response format (`"json"`, `"fish-ansi"`, `"fish-rendered"`)
- `is_last` (boolean, optional): Whether this is the last segment (affects delimiter rendering)

**Additional fields** (operation-specific):
- For `"register"`: `pid`, `shell`, `shell_version`

#### Response Message

Responses are operation-specific, but always valid JSON:

**Success response** (git operation):
```json
{
  "branch": "main",
  "ahead": 0,
  "behind": 0,
  "ahead_capped": false,
  "behind_capped": false,
  "staged": 2,
  "unstaged": 1,
  "untracked": 0,
  "conflicts": 0,
  "state": "dirty"
}
```

**Success response** (language operation):
```json
{
  "language": "Rust",
  "version": "1.90.0",
  "confidence": 0.95
}
```

**Success response** (ping operation):
```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

**Error response**:
```json
{
  "error": "Not in a git repository",
  "code": "NOT_A_REPO"
}
```

### Operations

#### 1. Git Status (`"git"`)

**Request**:
```json
{
  "op": "git",
  "cwd": "/path/to/repo",
  "format": "json",
  "is_last": false
}
```

**Response** (success):
```json
{
  "branch": "main",
  "ahead": 2,
  "behind": 1,
  "ahead_capped": false,
  "behind_capped": false,
  "staged": 3,
  "unstaged": 1,
  "untracked": 2,
  "conflicts": 0,
  "state": "dirty"
}
```

**State values**:
- `"clean"`: No changes
- `"dirty"`: Has changes (staged/unstaged/untracked)
- `"merging"`: In merge conflict resolution
- `"rebasing"`: In rebase operation
- `"cherry-picking"`: In cherry-pick operation
- `"reverting"`: In revert operation
- `"bisecting"`: In git bisect

**Error cases**:
- `"NOT_A_REPO"`: Not in a git repository
- `"PERMISSION_DENIED"`: Can't read .git directory
- `"INVALID_PATH"`: Path validation failed

#### 2. Language Detection (`"language"`)

**Request**:
```json
{
  "op": "language",
  "cwd": "/path/to/project",
  "format": "json"
}
```

**Response** (success):
```json
{
  "language": "Python",
  "version": "3.11.0",
  "confidence": 0.87
}
```

**Response** (no language detected):
```json
{
  "language": null,
  "version": null,
  "confidence": 0.0
}
```

#### 3. Client Registration (`"register"`)

**Purpose**: Register a shell process for live updates (SIGURG doorbell)

**Request**:
```json
{
  "op": "register",
  "pid": 12345,
  "cwd": "/home/user",
  "shell": "fish",
  "shell_version": "3.6.0"
}
```

**Response**:
```json
{
  "status": "registered",
  "watching": ["/home/user"]
}
```

**Behavior**:
- Agent tracks PID in client registry
- Agent starts watching `cwd` for git changes (if git repo)
- Agent sends SIGURG to PID when git status changes (repaint)
- Agent writes `<pid>.reload` then sends SIGURG when config/theme changes

#### 4. Ping (`"ping"`)

**Purpose**: Health check, verify agent is responsive

**Request**:
```json
{
  "op": "ping"
}
```

**Response**:
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

### Protocol Guarantees

**Ordering**: Requests are processed in order per connection (FIFO)

**Timeout**: Agent responses within 100ms (or returns partial/cached data)
- Git operations use progressive timeout (see Git Status Request flow)
- If git query takes > 100ms, agent returns cached data (if available)
- Client receives either: fresh data, stale data, or error

**Message Size Limits**:
- Request: 64KB max (enforced by agent)
- Response: Unlimited (but typically < 1KB)

**Connection Lifecycle**:
1. Client opens socket connection
2. Client sends request(s)
3. Agent responds to each request
4. Client closes connection (or keeps alive for multiple requests)
5. Agent cleans up on connection close

**Error Handling**:
- Malformed JSON → Agent closes connection
- Invalid operation → Agent returns error response
- Invalid path → Agent returns error response
- Timeout → Agent returns cached data or error

### Shell Integration Examples

#### Fish

```fish
function __gpy_request -a operation cwd format is_last
    # Build JSON request
    set -l socket "$XDG_RUNTIME_DIR/gpy-agent.sock"
    set -l request "{\"op\":\"$operation\",\"cwd\":\"$cwd\",\"format\":\"$format\",\"is_last\":$is_last}"

    # Send request, read response
    echo $request | socat - UNIX-CONNECT:$socket
end

# Usage
set -l git_status (__gpy_request "git" (pwd) "json" false)
```

#### Zsh

```zsh
__gpy_request() {
    local op=$1 cwd=$2 format=$3 is_last=$4
    local socket="${XDG_RUNTIME_DIR:-/tmp}/gpy-agent.sock"
    local request="{\"op\":\"$op\",\"cwd\":\"$cwd\",\"format\":\"$format\",\"is_last\":$is_last}"

    # Prefer the zsh/net/socket builtin (no external tool needed); fall back
    # to socat, then nc -U.
    zmodload zsh/net/socket 2>/dev/null
    if zsocket "$socket" 2>/dev/null; then
        echo $request >&$REPLY
        cat <&$REPLY
        exec {REPLY}>&-
    elif command -v socat &>/dev/null; then
        echo $request | socat - UNIX-CONNECT:$socket
    else
        echo $request | nc -U "$socket"
    fi
}
```

#### Bash

```bash
__gpy_request() {
    local op=$1 cwd=$2 format=$3 is_last=$4
    local socket="${XDG_RUNTIME_DIR:-/tmp}/gpy-agent.sock"
    local request="{\"op\":\"$op\",\"cwd\":\"$cwd\",\"format\":\"$format\",\"is_last\":$is_last}"

    # Bash has no /dev/tcp-to-Unix-socket path; use socat, then nc -U.
    if command -v socat &>/dev/null; then
        echo "$request" | socat - UNIX-CONNECT:"$socket"
    else
        echo "$request" | nc -U "$socket"
    fi
}
```

### Protocol Versioning

**Current version**: 1.0 (implicit, no version field in protocol)

**Future versioning**:
```json
{
  "version": "2.0",
  "op": "git",
  ...
}
```

**Compatibility strategy**:
- Agent supports multiple protocol versions concurrently
- Version negotiation via `version` field in request
- If no `version` field → assume 1.0
- Agent responds with same version as request

**Breaking changes** (require version bump):
- Renaming fields
- Changing field types
- Removing operations

**Non-breaking changes** (no version bump needed):
- Adding new fields (ignored by old clients)
- Adding new operations
- Adding new response formats

### Performance Characteristics

**Latency breakdown** (cached git status):
```
Socket open:      ~0.005ms
Request write:    ~0.005ms
Agent processing: ~0.020ms  (cache lookup)
Response read:    ~0.005ms
Socket close:     ~0.005ms
─────────────────────
Total:            ~0.04ms
```

**Latency breakdown** (fresh git status):
```
Socket open:      ~0.005ms
Request write:    ~0.005ms
Agent processing: ~31-45ms (git status --porcelain=v2 subprocess; the
                           per-repo-size figures live in the module docs at
                           the top of gpy-agent/src/git/native/mod.rs)
Response read:    ~0.005ms
Socket close:     ~0.005ms
─────────────────────
Total:            ~31-45ms (cold; a fresh cache entry returns without
                           spawning git at all)
```

**Throughput**:
- Single connection: 1000+ requests/second (with caching)
- Concurrent connections: Limited by OS (typically 128+ simultaneous)

**Connection pooling benefits**:
- Reusing connections saves 0.2ms per request (no open/close overhead)
- Fish shells typically make 5-10 requests per prompt
- Savings: 1-2ms per prompt

### Security Considerations

**Threat model**:
- **Trusted**: User's own processes (same UID)
- **Untrusted**: Other users on the system

**Protections**:

1. **Socket Permissions** (`0600`):
   - Only owner can connect
   - OS enforces UID matching
   - No cross-user access

2. **Path Validation**:
   - Rejects paths with null bytes, control characters
   - Limits path length to prevent buffer exploits
   - Canonicalizes paths to prevent traversal attacks

3. **Message Size Limits**:
   - 64KB max request size
   - Prevents memory exhaustion DoS

4. **Rate Limiting**:
   - 50ms minimum interval per client
   - Prevents request flooding

5. **PID Validation** (for registration):
   - `kill(pid, 0)` checks if PID is valid and owned by user
   - Returns `EPERM` if PID belongs to different user
   - Prevents signal delivery to arbitrary processes

**Attack scenarios prevented**:
- ✅ Malicious IPC client sending crafted requests
- ✅ PID spoofing to receive signals for other user's shells
- ✅ Socket hijacking by other users
- ✅ Path traversal to read arbitrary files
- ✅ DoS via message flooding or large messages

**Not protected against**:
- ❌ User attacking their own agent (same UID = full trust)
- ❌ Root user accessing any socket (by design)

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
  ~31ms on a 200-file repo, ~45ms on a 91k-file repo

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

### Threat Model

**Trusted**: User's own processes (same UID)
**Untrusted**: Other users on the system

**Attack scenarios prevented**:
1. **Malicious IPC client**: Path validation, message size limits, rate limiting
2. **PID spoofing**: OS enforces via kill() permissions (EPERM if not your PID)
3. **Socket hijacking**: 0600 permissions, only owner can connect
4. **Path traversal**: Security module validates all paths
5. **DoS**: 64KB message limit, rate limiting (50ms per client)

### Defense Layers

```
IPC Request
  ↓
1. Socket permissions (0600) → only user can connect
  ↓
2. Message size validation (64KB max)
  ↓
3. JSON parsing (reject malformed)
  ↓
4. Path validation (length, null bytes, control chars)
  ↓
5. PID validation (kill(pid, 0) → ESRCH if invalid)
  ↓
6. Rate limiting (50ms minimum interval per client)
  ↓
Process request
```

---

## Performance Characteristics

### Latency Targets

| Operation | Target | Typical | Notes |
|-----------|--------|---------|-------|
| IPC round-trip | < 1.5ms | ~0.04ms | Cached response |
| Git status (fresh) | < 100ms | ~17ms | Depends on repo size |
| Prompt render | < 50ms | 10-20ms | All segments |
| Signal delivery | < 10ms | 2-5ms | SIGURG to all clients |

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
socket_path = "/tmp/gpy.sock"  # Override default

[git]
timeout_seconds = 5
skip_paths = ["/mnt/slow-drive"]

[language]
cache_ttl_seconds = 300

[ui]
show_icons = true

[clock]
show_seconds = false  # false = 60s updates, true = 1s updates
```

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

Hot-reload functionality is now comprehensively tested to prevent regressions:

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
just lint   # moon run gpy-agent:clippy

# Fish integration tests
./scripts/test_fish.sh

# Interactive sessions on a real pseudo-terminal (idle repaint, two shells,
# a config reload reaching a Fish client)
fish tests/fish/e2e_interactive_session.test.fish
```

---

## Common Operations

### Adding a New Segment

See [segment-development.md](segment-development.md) for detailed guide.

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

# IPC latency
socat -T 1 - UNIX-CONNECT:/tmp/gpy.sock <<< '{"op":"ping"}'

# Git query timing
time git status --porcelain
```

---

## Next Steps for New Contributors

1. **Read this doc** (you're here! ✓)
2. **Read module docs**: `gpy-agent/src/agent.rs`, `gpy-agent/src/watcher/mod.rs`
3. **Trace a request**: Follow git status request through entire stack
4. **Run tests**: `cargo test`, `./scripts/test_fish.sh`
5. **Read [segment-development.md](segment-development.md)**: Learn how to add segments
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

- [modules.md](../archive/modules.md) - Detailed module reference (archived)
- [segment-development.md](segment-development.md) - How to add segments
- [caching-strategy.md](../archive/caching-strategy.md) - Cache invalidation deep dive (archived)
- [ADR-0004: SIGUSR1 Live Updates](adr/adr-0004-sigusr1-live-updates.md) - Why the watcher signals shells instead of polling
- [ADR-0007: SIGURG Doorbell](adr/adr-0007-sigurg-doorbell-notifications.md) - The current single-signal protocol and flag files
- [CONTRIBUTING.md § Quality Gates](../../CONTRIBUTING.md#quality-gates) - How to run every suite
- [concepts/git_status.concept.toml](concepts/git_status.concept.toml) - Example formal concept specification (demonstrates LLM-optimized module specs; other modules intentionally omitted to avoid over-engineering)

---

**Questions?** Open an issue with the `question` label or ask in discussions.
