# Segment Development Guide

**Audience**: Contributors adding new prompt segments to GPY.

**Prerequisites**:
- Read [architecture.md](architecture.md) for system overview
- Read [modules.md](../archive/modules.md) for module references (archived)

This guide walks through adding a new segment to the GPY prompt system, from Rust agent implementation to Fish shell rendering.

---

## Table of Contents

- [Segment Architecture](#segment-architecture)
- [Example: Adding a Kubernetes Context Segment](#example-adding-a-kubernetes-context-segment)
- [Step-by-Step Implementation](#step-by-step-implementation)
- [Testing Your Segment](#testing-your-segment)
- [Configuration and Theming](#configuration-and-theming)
- [Performance Considerations](#performance-considerations)
- [Common Patterns](#common-patterns)

---

## Segment Architecture

### Two-Part Design

GPY segments follow a **detect/render split**:

1. **Detection (Rust agent)**: Fast detection logic runs in background daemon
2. **Rendering (Fish shell)**: Lightweight formatting in Fish shell

**Why this split?**
- Heavy work (file I/O, subprocess calls) happens in Rust with caching
- Fish shell just formats pre-computed data (fast, no blocking)
- Segments can be enabled/disabled without restarting agent

### Data Flow

```
┌─────────────────────┐
│  Fish Shell (cwd)   │
└──────────┬──────────┘
           │ IPC request (cwd=/home/user/project)
           ▼
┌─────────────────────┐
│   Rust Agent        │
│  ┌───────────────┐  │
│  │ Detection     │  │  1. Check cache
│  │ Logic         │  │  2. Run detection (if cache miss)
│  └───────────────┘  │  3. Store in cache
│  ┌───────────────┐  │
│  │ Response      │  │  Return structured data
│  │ Formatting    │  │  (JSON, fish-ansi, or fish-source)
│  └───────────────┘  │
└──────────┬──────────┘
           │ IPC response
           ▼
┌─────────────────────┐
│  Fish Shell         │
│  ┌───────────────┐  │
│  │ Segment       │  │  1. Parse response
│  │ Renderer      │  │  2. Apply colors/icons
│  └───────────────┘  │  3. Emit ANSI codes
└─────────────────────┘
```

### Segment Components

Every segment has three parts:

1. **Rust detection** (`gpy-agent/src/<subsystem>/`)
   - Fast detection logic with caching
   - Returns structured data (enum or struct)

2. **IPC operation** (`gpy-agent/src/ipc/protocol.rs`)
   - New `Operation` variant
   - Response data type

3. **Fish renderer** (`gpy-fish/functions/segments/__gpy_segment_<name>.fish`)
   - Parse response data
   - Render with colors/icons
   - Return formatted string

---

## Example: Adding a Kubernetes Context Segment

Let's add a segment that shows the current Kubernetes context and namespace.

> Two complete, commented Fish-side examples ship with the docs rather than
> the prompt: `docs/dev/examples/segments/example_custom.fish` (a battery
> segment) and `example_docker.fish`. Copy one into `fish/segments/` (or a
> plugin, see [plugins.md](plugins.md)) to start from a working file.
>
> This segment is a worked example, not a shipped one: there is no
> `gpy-agent/src/k8s/` module or `k8s_tests.rs` in the repository. Every
> file the rest of this guide tells you to create is the one *you* would add
> for a new segment; the shipped segments to compare against live in
> `gpy-agent/src/` (git, language, directory, ...) and `fish/segments/`.

**Goal**: Display `⎈ prod-cluster:default` in the prompt when inside a Kubernetes project.

### Preview of What We'll Build

**Rust Detection**:
```rust
pub struct K8sInfo {
    pub context: String,
    pub namespace: String,
}

pub fn detect_k8s(path: &Path) -> Result<Option<K8sInfo>> {
    // Read ~/.kube/config and parse current context
}
```

**Fish Rendering**:
```fish
function __gpy_segment_k8s
    # Input: JSON like {"context":"prod-cluster","namespace":"default"}
    # Output: " ⎈ prod-cluster:default "
end
```

---

## Step-by-Step Implementation

### Step 1: Add Detection Logic (Rust)

Create new module: `gpy-agent/src/k8s/detector.rs`

```rust
//! Kubernetes context detection

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Kubernetes context and namespace information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sInfo {
    pub context: String,
    pub namespace: String,
}

/// Detect Kubernetes context for the current directory
///
/// Checks if the directory contains Kubernetes manifests (*.yaml with kind: *)
/// and returns the current context from ~/.kube/config
pub fn detect_k8s(path: &Path) -> Result<Option<K8sInfo>> {
    // Skip detection if no k8s manifests in directory
    if !has_k8s_manifests(path) {
        return Ok(None);
    }

    // Read kubeconfig
    let config_path = kubeconfig_path()?;
    let config_content = fs::read_to_string(&config_path)
        .map_err(|e| Error::config(format!("Failed to read kubeconfig: {}", e)))?;

    // Parse current context (simplified - production code would use a YAML parser)
    let context = parse_current_context(&config_content)?;
    let namespace = parse_current_namespace(&config_content, &context)?;

    Ok(Some(K8sInfo { context, namespace }))
}

fn has_k8s_manifests(path: &Path) -> bool {
    // Check for common k8s files
    path.join("kubernetes").exists()
        || path.join("k8s").exists()
        || path.join("deployment.yaml").exists()
        || path.join("kustomization.yaml").exists()
}

fn kubeconfig_path() -> Result<PathBuf> {
    // Check KUBECONFIG env var, fall back to ~/.kube/config
    if let Ok(env_path) = std::env::var("KUBECONFIG") {
        return Ok(PathBuf::from(env_path));
    }

    let home = std::env::var("HOME")
        .map_err(|_| Error::config("HOME not set"))?;
    Ok(PathBuf::from(home).join(".kube/config"))
}

fn parse_current_context(yaml: &str) -> Result<String> {
    // Simplified parser - production should use serde_yaml
    for line in yaml.lines() {
        if let Some(context) = line.strip_prefix("current-context: ") {
            return Ok(context.trim().to_string());
        }
    }
    Err(Error::config("No current-context in kubeconfig"))
}

fn parse_current_namespace(yaml: &str, context: &str) -> Result<String> {
    // Simplified - would parse contexts section in production
    Ok("default".to_string())  // Fallback
}
```

**Create module declaration**: Add to `gpy-agent/src/lib.rs`:

```rust
pub mod k8s;
```

**Export detection function**: Create `gpy-agent/src/k8s/mod.rs`:

```rust
pub mod detector;
pub use detector::{detect_k8s, K8sInfo};
```

### Step 2: Add IPC Operation (Rust)

**Edit `gpy-agent/src/ipc/protocol.rs`**:

Add operation variant:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Ping,
    Git,
    Language,
    K8s,  // <-- Add this
    Shutdown,
    ThemeExport,
}
```

Add response data type:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseData {
    Ping(String),
    Git(GitInfo),
    Language(Option<LanguageInfo>),
    K8s(Option<K8sInfo>),  // <-- Add this
    ThemeExport(String),
}
```

### Step 3: Add Request Handler (Rust)

**Edit `gpy-agent/src/agent.rs`**:

Add handler function:
```rust
fn handle_k8s_request(state: &AgentState, path: &Path) -> Result<Option<K8sInfo>> {
    // Try cache first
    if let Some(cached) = state.k8s_cache.get(path) {
        return Ok(Some(cached));
    }

    // Cache miss - run detection
    let info = crate::k8s::detect_k8s(path)?;

    // Cache result if found
    if let Some(ref k8s_info) = info {
        state.k8s_cache.insert(path.to_path_buf(), k8s_info.clone());
    }

    Ok(info)
}
```

**Wire up in IPC server**: Edit `gpy-agent/src/ipc/server.rs` (around line 250):

```rust
Operation::K8s => {
    let info = agent::handle_k8s_request(&agent, &cwd)?;
    ResponseData::K8s(info)
}
```

### Step 4: Add Cache (Optional but Recommended)

**Create `gpy-agent/src/k8s/cache.rs`**:

```rust
use crate::k8s::K8sInfo;
use lru::LruCache;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct K8sCache {
    cache: Arc<Mutex<LruCache<PathBuf, CachedK8sInfo>>>,
    ttl: Duration,
}

struct CachedK8sInfo {
    info: K8sInfo,
    cached_at: Instant,
}

impl K8sCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            ttl,
        }
    }

    pub fn get(&self, path: &Path) -> Option<K8sInfo> {
        let mut cache = self.cache.lock().ok()?;
        let entry = cache.get(path)?;

        // Check if expired
        if entry.cached_at.elapsed() > self.ttl {
            cache.pop(path);
            return None;
        }

        Some(entry.info.clone())
    }

    pub fn insert(&self, path: PathBuf, info: K8sInfo) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(path, CachedK8sInfo {
                info,
                cached_at: Instant::now(),
            });
        }
    }
}
```

**Add cache to agent state**: Edit `gpy-agent/src/agent.rs`:

```rust
pub struct AgentState {
    pub config: Arc<Config>,
    pub git_cache: Arc<GitCache>,
    pub lang_cache: Arc<LanguageCache>,
    pub k8s_cache: Arc<K8sCache>,  // <-- Add this
    pub client_directory: Arc<ClientDirectory>,
}
```

### Step 5: Add Fish Renderer

**Create `gpy-fish/functions/segments/__gpy_segment_k8s.fish`**:

```fish
function __gpy_segment_k8s
    # Parse response from agent
    # Expected format: {"context":"prod-cluster","namespace":"default"}

    set -l context (string match -r '"context":"([^"]+)"' -- $argv[1])[2]
    set -l namespace (string match -r '"namespace":"([^"]+)"' -- $argv[1])[2]

    # Return empty if no context detected
    if test -z "$context"
        return
    end

    # Get colors from theme
    set -l icon_color $__color_k8s_icon
    set -l context_color $__color_k8s_context
    set -l namespace_color $__color_k8s_namespace
    set -l bg_color $__color_k8s_bg

    # Build segment with icon and formatted text
    set -l icon "⎈"
    set -l content "$context:$namespace"

    # Output with ANSI color codes
    printf '%s' (set_color -b $bg_color)(set_color $icon_color)" $icon "
    printf '%s' (set_color $context_color)"$context"
    printf '%s' (set_color $namespace_color)":$namespace "
    printf '%s' (set_color normal)
end
```

### Step 6: Add Segment to Prompt

**Edit `gpy-fish/functions/__gpy_prompt.fish`**:

Add to segment order (around line 50):
```fish
set -l segments directory git language k8s clock duration status
```

Add segment renderer call (around line 100):
```fish
case k8s
    if set -q __gpy_k8s_enabled
        __gpy_request k8s $PWD | __gpy_segment_k8s
    end
```

### Step 7: Add Configuration

**Edit `gpy-agent/src/config/schema.rs`**:

Add segment config:
```rust
pub struct SegmentConfig {
    pub order: Vec<String>,
    pub git_enabled: bool,
    pub language_enabled: bool,
    pub k8s_enabled: bool,  // <-- Add this
    // ... rest
}
```

**Edit user config template** (`gpy-fish/config/gpy.toml`):

```toml
[segments]
order = ["directory", "git", "language", "k8s", "clock", "duration", "status"]
k8s_enabled = true
```

**Add theme colors** (`gpy-fish/config/theme.toml`):

```toml
[colors.k8s]
icon = "#326ce5"        # Kubernetes blue
context = "#ffffff"
namespace = "#aaaaaa"
bg = "#073642"
```

### Step 8: Add File Watcher Support (Optional)

If the segment should update when files change:

**Edit `gpy-agent/src/watcher/mod.rs`** (around line 329):

Add file pattern:
```rust
"kubeconfig" | ".kube/config" => FileEvent::K8s {
    path: path.to_path_buf(),
},
```

Add event variant:
```rust
pub enum FileEvent {
    Git { paths: GitPaths },  // coalesced set of paths, see watcher::GitPaths
    Language { path: PathBuf },
    K8s { path: PathBuf },  // <-- Add this
    Config { path: PathBuf },
}
```

**Update watcher callback** in `agent.rs` (around line 200):

```rust
FileEvent::K8s { path } => {
    // Invalidate cache
    agent.k8s_cache.invalidate(&path);

    // Signal clients in this directory
    agent.client_directory.signal_clients_for_repo(&path, config.throttle_ms())?;
}
```

---

## Testing Your Segment

### Unit Tests

**Create `gpy-agent/src/k8s/tests.rs`**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_k8s_with_manifest() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("deployment.yaml");
        fs::write(&manifest_path, "apiVersion: apps/v1\nkind: Deployment").unwrap();

        // Should detect k8s context
        let result = detect_k8s(temp.path()).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_detect_k8s_without_manifest() {
        let temp = TempDir::new().unwrap();

        // Should return None when no k8s files present
        let result = detect_k8s(temp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_cache_ttl() {
        let cache = K8sCache::new(10, Duration::from_millis(100));
        let path = PathBuf::from("/test");
        let info = K8sInfo {
            context: "test".to_string(),
            namespace: "default".to_string(),
        };

        // Insert and retrieve immediately
        cache.insert(path.clone(), info.clone());
        assert!(cache.get(&path).is_some());

        // Wait for TTL expiration
        std::thread::sleep(Duration::from_millis(150));
        assert!(cache.get(&path).is_none());
    }
}
```

### Integration Tests

**Create `gpy-agent/tests/k8s_tests.rs`** (a new file for the example segment; every `gpy-agent/tests/*.rs` file is its own nextest target):

```rust
use gpy_agent::k8s::detect_k8s;
use std::env;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_k8s_detection_integration() {
    let temp = TempDir::new().unwrap();

    // Create k8s manifest
    fs::write(
        temp.path().join("deployment.yaml"),
        r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: test-app
"#,
    )
    .unwrap();

    // Set up test kubeconfig
    let kubeconfig = temp.path().join("kubeconfig");
    fs::write(
        &kubeconfig,
        r#"
current-context: test-cluster
contexts:
- name: test-cluster
  context:
    cluster: test-cluster
    namespace: default
"#,
    )
    .unwrap();

    env::set_var("KUBECONFIG", &kubeconfig);

    // Run detection
    let result = detect_k8s(temp.path()).unwrap();
    assert!(result.is_some());

    let info = result.unwrap();
    assert_eq!(info.context, "test-cluster");
    assert_eq!(info.namespace, "default");
}
```

### Manual Testing

```bash
# Build agent with new segment
cd gpy-agent
cargo build --release

# Install to ~/.local/bin
cp target/release/gpy-agent ~/.local/bin/
cp target/release/gpy ~/.local/bin/

# Restart agent
gpy restart

# Create test directory with k8s manifest
mkdir -p /tmp/k8s-test
cd /tmp/k8s-test
echo "apiVersion: v1\nkind: Pod" > pod.yaml

# Check if segment appears in prompt
fish -c "fish_prompt"

# Test cache invalidation
touch ~/.kube/config
# Prompt should update within 150ms
```

### Fish Function Testing

```fish
# Test segment renderer directly
set -g __color_k8s_icon "#326ce5"
set -g __color_k8s_context "#ffffff"
set -g __color_k8s_namespace "#aaaaaa"
set -g __color_k8s_bg "#073642"

# Simulate agent response
echo '{"context":"prod-cluster","namespace":"default"}' | __gpy_segment_k8s

# Expected output: colored " ⎈ prod-cluster:default "
```

---

## Configuration and Theming

### Segment Configuration

Users can enable/disable segments in `~/.config/gpy/config.toml`:

```toml
[segments]
order = ["directory", "git", "language", "k8s", "clock", "duration", "status"]

# Enable/disable segments
git_enabled = true
language_enabled = true
k8s_enabled = true
```

### Theme Configuration

Colors and icons in `~/.config/gpy/themes/<theme>.toml` are grouped under `[segments.*]`:

```toml
[segments.k8s]
icon_color = "#326ce5"     # Kubernetes blue
context_color = "#ffffff"  # White
namespace_color = "#aaaaaa"# Gray
bg_color = "#073642"       # Dark background

[segments.k8s.open]
icon = "["
icon_color = "match_text"
bg_color = "match_bg"
```

### Default Values

Provide sensible defaults in code:

```rust
impl Default for K8sTheme {
    fn default() -> Self {
        Self {
            icon_color: "#326ce5".into(),
            context_color: "#ffffff".into(),
            namespace_color: "#aaaaaa".into(),
            bg_color: "#073642".into(),
            prompt_icon: Some("⎈".into()),
        }
    }
}
```

---

## Performance Considerations

### Cache Strategy

**Rule of thumb**: Cache results that are expensive to compute.

**For K8s segment**:
- **Capacity**: 50 directories (most users work in <10 projects)
- **TTL**: 300 seconds (context changes infrequently)
- **Cooldown**: Not needed (no file watcher)

```rust
K8sCache::new(50, Duration::from_secs(300))
```

### Detection Optimization

**Fast path**: Check for k8s files before reading kubeconfig

```rust
pub fn detect_k8s(path: &Path) -> Result<Option<K8sInfo>> {
    // Early return if no k8s files (saves file I/O)
    if !has_k8s_manifests(path) {
        return Ok(None);
    }

    // Only read kubeconfig if needed
    let config = read_kubeconfig()?;
    parse_k8s_info(config)
}
```

**Benchmark target**: Detection should complete in <20ms

### Async Considerations

**Question**: Should detection be async?

**Answer**: Only if it makes network calls or runs long-lived subprocesses.

- **Sync OK**: Reading local files (<10ms)
- **Async preferred**: HTTP API calls (>50ms)

**Example async detection**:

```rust
pub async fn detect_k8s_async(path: &Path) -> Result<Option<K8sInfo>> {
    // Read kubeconfig asynchronously
    let config = tokio::fs::read_to_string(kubeconfig_path()?).await?;

    // Parse context
    Ok(Some(parse_k8s_info(&config)?))
}
```

---

## Common Patterns

### Pattern 1: File-Based Detection

**Use case**: Detect based on presence of specific files

```rust
pub fn detect_by_files(path: &Path, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| path.join(pattern).exists())
}

// Usage
if detect_by_files(path, &["package.json", "node_modules"]) {
    // Node.js project detected
}
```

### Pattern 2: Command Version Detection

**Use case**: Run command to get version info

```rust
use std::process::Command;

pub fn get_version(command: &str) -> Result<Option<String>> {
    let output = Command::new(command)
        .arg("--version")
        .output()
        .map_err(|_| Error::language("Command not found"))?;

    if !output.status.success() {
        return Ok(None);
    }

    let version = String::from_utf8_lossy(&output.stdout);
    Ok(Some(parse_version(&version)))
}
```

**Performance note**: Cache results! Version checks are slow (50-200ms).

### Pattern 3: Environment Variable Detection

**Use case**: Detect based on environment variables

```rust
pub fn detect_from_env(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

// Usage
if let Some(node_version) = detect_from_env("NODE_VERSION") {
    // Use specified version
}
```

### Pattern 4: Conditional Rendering

**Use case**: Only show segment in specific conditions

```fish
function __gpy_segment_k8s
    # Only show if in k8s directory
    if not test -d kubernetes; and not test -f deployment.yaml
        return
    end

    # ... render segment
end
```

### Pattern 5: Multi-Line Segments

**Use case**: Complex segments with multiple lines

```fish
function __gpy_segment_k8s_detailed
    # First line: context
    printf '%s\n' (set_color $context_color)"Context: $context"

    # Second line: namespace
    printf '%s\n' (set_color $namespace_color)"Namespace: $namespace"
end
```

---

## Troubleshooting

### Segment Not Appearing

**Checklist**:
1. ✅ Added to `segments.order` in config?
2. ✅ Segment enabled: `k8s_enabled = true`?
3. ✅ IPC operation wired up in `ipc/server.rs`?
4. ✅ Fish function exists: `__gpy_segment_k8s.fish`?
5. ✅ Agent restarted: `gpy restart`?

**Debug**:
```fish
# Test IPC directly
printf '{"op":"k8s","cwd":"%s","format":"json"}\n' $PWD | socat - UNIX-CONNECT:~/.cache/gpy/gpy-agent.sock

# Expected: {"status":"ok","data":{"context":"...","namespace":"..."}}
```

### Segment Rendering Issues

**Checklist**:
1. ✅ Theme colors defined in `theme.toml`?
2. ✅ Colors exported to Fish: `set -gx __color_k8s_icon ...`?
3. ✅ Fish syntax valid: `fish -n __gpy_segment_k8s.fish`?

**Debug**:
```fish
# Print all theme variables
set -g | grep __color_k8s

# Test renderer with mock data
echo '{"context":"test","namespace":"default"}' | __gpy_segment_k8s
```

### Performance Issues

**Symptoms**: Prompt feels slow, >50ms render time

**Diagnose**:
```fish
# Time segment rendering
time fish -c "__gpy_request k8s $PWD | __gpy_segment_k8s"

# Should be <10ms (cached), <50ms (uncached)
```

**Fix**:
- Add caching if not present
- Optimize detection (avoid subprocess calls)
- Use async for network/slow operations

---

## Common Mistakes

Based on 15+ segment implementations, here are the most frequent pitfalls and how to avoid them:

### 1. Forgetting to Add Segment to `__enabled_segments`

**Symptom**: Segment code exists but never appears in prompt.

**Why it happens**: The Fish prompt loop only renders segments listed in `__enabled_segments`.

**Fix**:
```fish
# In core/init.fish (look for the __enabled_segments initialization)
set -g __enabled_segments clock duration language directory git your_segment
```

**How to verify**: `echo $__enabled_segments` should list your segment.

**Note**: The segment list is set during initialization in `core/init.fish`. The order determines left-to-right rendering in the prompt.

---

### 2. Expensive Operations in `detect()` Function

**Symptom**: Prompt feels sluggish, takes >50ms to render.

**Why it happens**: Detection runs synchronously in the prompt render path. Slow operations (API calls, subprocess spawning, large file reads) block the entire prompt.

**Fix - Option A** (Fish-only, fast path):
```fish
function segment_docker_detect
    # ✅ GOOD: Quick check (< 5ms)
    command -v docker >/dev/null 2>&1 || return 1
    test -f .dockerenv || return 1
    return 0
end
```

**Fix - Option B** (Move to agent if unavoidably slow):
```rust
// In gpy-agent/src/docker/detector.rs
pub fn detect_docker(path: &Path) -> Result<Option<DockerInfo>> {
    // Heavy work happens here with caching
    // Fish just requests via IPC
}
```

**Benchmark target**: `detect()` should complete in <10ms. Use `time` to measure:
```fish
time segment_your_segment_detect
```

---

### 3. Not Checking if Command Exists

**Symptom**: Error spam in agent logs or Fish stderr when command not installed.

**Why it happens**: Segment assumes tool is available without checking.

**Wrong**:
```fish
function segment_kubectl_render
    # ❌ BAD: Crashes if kubectl not installed
    set -l context (kubectl config current-context)
    printf '%s' $context
end
```

**Right**:
```fish
function segment_kubectl_render
    # ✅ GOOD: Check first
    command -v kubectl >/dev/null 2>&1 || return

    set -l context (kubectl config current-context 2>/dev/null)
    test -z "$context"; and return

    printf '%s' $context
end
```

**Pattern**: Always guard subprocess calls with `command -v` and redirect stderr.

---

### 4. Not Handling Empty/Error Output

**Symptom**: Segment shows garbage, empty boxes, or error messages in prompt.

**Why it happens**: Command fails or returns empty output, but segment renders anyway.

**Wrong**:
```fish
function segment_aws_render
    # ❌ BAD: No error handling
    set -l profile (aws configure get profile)
    printf 'AWS: %s' $profile  # Shows "AWS: " if command fails
end
```

**Right**:
```fish
function segment_aws_render
    # ✅ GOOD: Validate output before rendering
    set -l profile (aws configure get profile 2>/dev/null)

    # Return early if no profile
    test -z "$profile"; and return

    # Return early if default (not interesting)
    test "$profile" = "default"; and return

    printf 'AWS: %s' $profile
end
```

**Pattern**: Check output is non-empty before rendering. Return silently on errors.

---

### 5. Skipping Tests

**Symptom**: Segment breaks on edge cases (empty repos, missing files, network errors).

**Why it happens**: Manual testing only covers happy path.

**Minimum test coverage**:
```rust
// gpy-agent/tests/your_segment_tests.rs
#[test]
fn test_segment_with_valid_data() { /* ... */ }

#[test]
fn test_segment_with_missing_file() { /* ... */ }

#[test]
fn test_segment_with_invalid_data() { /* ... */ }

#[test]
fn test_cache_hit() { /* ... */ }

#[test]
fn test_cache_miss() { /* ... */ }
```

**Fish integration test**:
```fish
# tests/fish/test_your_segment.fish
# Test segment appears when expected
# Test segment hidden when not applicable
# Test error handling
```

---

### 6. Hardcoding Colors/Icons in Fish Functions

**Symptom**: Users can't customize segment appearance via theme.

**Why it happens**: Colors baked into segment code instead of using theme variables.

**Wrong**:
```fish
function segment_git_render
    # ❌ BAD: Hardcoded colors
    printf '%s' (set_color blue)"  main"(set_color normal)
end
```

**Right**:
```fish
function segment_git_render
    # ✅ GOOD: Use theme variables
    set -l branch_color $__color_git_branch
    set -l icon $__glyph_git_branch
    printf '%s' (set_color $branch_color)"$icon main"(set_color normal)
end
```

**Fallback pattern** (if theme not loaded):
```fish
set -l color $__color_git_branch
test -z "$color"; and set color blue  # Fallback
```

---

### 7. Not Testing Agent vs Oneshot Modes

**Symptom**: Works with daemon, fails in oneshot mode (or vice versa).

**Why it happens**: Different code paths, different error handling.

**Test both modes**:
```bash
# Test with daemon
gpy start
fish -c "fish_prompt"

# Test with oneshot (daemon stopped)
gpy stop
fish -c "fish_prompt"

# Should work in both modes (may be slower in oneshot)
```

**Fish fallback pattern**:
```fish
function __gpy_request
    # Try daemon first
    if __gpy_daemon_available
        __gpy_ipc_request $argv
    else
        # Fall back to oneshot
        gpy-agent oneshot $argv
    end
end
```

---

### 8. Ignoring Cache Invalidation

**Symptom**: Segment shows stale data after file changes.

**Why it happens**: Cache never invalidates, even when underlying data changes.

**Fix**: Add file watcher pattern in agent:
```rust
// gpy-agent/src/watcher/filesystem.rs
match file_name {
    "your-config-file.yaml" => {
        FileEvent::YourSegment { path: path.to_path_buf() }
    }
    // ...
}

// gpy-agent/src/agent.rs
FileEvent::YourSegment { path } => {
    your_segment_cache.invalidate(&path);
    client_registry.notify_sigusr1(Some(&path))?;
}
```

**Alternative**: Use time-based TTL if file watching not critical:
```rust
YourSegmentCache::new(50, Duration::from_secs(300))  // 5 min TTL
```

---

### 9. Breaking on Special Characters in Paths/Data

**Symptom**: Segment crashes or shows garbage when repo path has spaces, quotes, or Unicode.

**Why it happens**: Insufficient input validation.

**Fix**: Validate all external input:
```rust
// In Rust agent
use crate::security::validate_path;

pub fn detect_your_segment(path: &Path) -> Result<Option<YourInfo>> {
    // Validate path first
    validate_path(path)?;

    // ... rest of detection
}
```

**Fish escaping**:
```fish
# Use proper quoting
set -l output (some-command "$PWD")  # ✅ Quoted
set -l output (some-command $PWD)    # ❌ Breaks on spaces
```

---

### 10. Not Documenting Configuration

**Symptom**: Users don't know segment is configurable, file issues for features that already exist.

**Fix**: Document all config options:

**In config.toml**:
```toml
[segments.your_segment]
enabled = true
cache_ttl_seconds = 300
timeout_seconds = 5
```

**In theme.toml**:
```toml
[segments.your_segment]
icon_color = "#ff0000"
text_color = "#ffffff"
bg_color = "#000000"

[segments.your_segment.icon]
icon = "🔧"
nerd_font = ""
```

**In README or [theme-customization.md](../user/theme-customization.md)**:
```markdown
### Your Segment

Shows XYZ in prompt when ABC conditions are met.

Configuration:
- `segments.your_segment.enabled` - Enable/disable (default: true)
- `segments.your_segment.cache_ttl_seconds` - Cache duration (default: 300)
```

---

## Quick Checklist for New Segments

Before submitting:
- [ ] Added to `__enabled_segments` in Fish prompt
- [ ] Detection completes in <10ms (or uses agent)
- [ ] Handles missing commands gracefully (`command -v`)
- [ ] Returns early on empty/error output
- [ ] Uses theme variables for colors/icons (no hardcoded values)
- [ ] Tested in both daemon and oneshot modes
- [ ] Added unit tests (Rust) and integration tests (Fish)
- [ ] Cache invalidates on relevant file changes
- [ ] Handles special characters in paths/data
- [ ] Documented configuration options

---

## Best Practices

### DO:
- ✅ Cache expensive operations (file I/O, subprocesses)
- ✅ Return `None` early if detection not applicable
- ✅ Use structured data types (`struct` not `HashMap`)
- ✅ Write unit tests for detection logic
- ✅ Provide sensible default theme values
- ✅ Document configuration options

### DON'T:
- ❌ Make network calls without caching (>100ms latency)
- ❌ Run subprocesses on every prompt render
- ❌ Use blocking I/O in async context
- ❌ Forget to handle missing files gracefully
- ❌ Hardcode colors in Fish functions
- ❌ Skip error handling

---

## Next Steps

- **For module references**: See [modules.md](../archive/modules.md) (archived)
- **For cache tuning**: See [caching-strategy.md](../archive/caching-strategy.md) (archived)
- **For system architecture**: See [architecture.md](architecture.md)

---

## Example Segments to Study

**Simple file-based detection**:
- Language segment (`gpy-agent/src/language/detector.rs`)
- Look for patterns like `package.json`, `Cargo.toml`

**Subprocess version detection**:
- Language version detection (`gpy-agent/src/language/registry.rs`)
- Run `node --version`, `rustc --version`, etc.

**Complex state detection**:
- Git segment (`gpy-agent/src/git/commands.rs`)
- Parse `.git/` directory structure
- Handle merge/rebase/bisect states

**Real-time updates**:
- Clock segment (`gpy-agent/src/agent.rs:200-250`)
- Timer-based signal delivery
- No file watching needed
