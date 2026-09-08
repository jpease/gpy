# Tactic: Add New Segment to GPY Prompt

**Category**: Feature Development
**Difficulty**: ⭐⭐☆☆☆ (Beginner-Intermediate)
**Estimated Time**: 2-4 hours
**Success Rate**: 15/17 attempts (88%)
**Last Updated**: 2025-01-15
**Related**: [concepts/fish_functions.md], [failures/segment_not_appearing.md]

## Purpose

Add a new segment to the GPY Fish prompt (e.g., Docker context, Kubernetes cluster, Python virtualenv).

## When to Use

- Adding new environment information to prompt
- Extending GPY with custom status indicators
- Integrating third-party tool state

## Prerequisites

- Fish shell 3.6+
- Understanding of Fish functions
- GPY installed and working
- (Optional) Rust knowledge if agent support needed

## Step-by-Step Process

### Phase 1: Decide Scope (10 minutes)

**Question 1**: Does this segment need the Rust agent?

- **No (Fish-only)**: If data is quickly obtainable via Fish builtins
  - Examples: Environment variable, simple file check, Fish variable
  - Pros: Simpler, no IPC overhead
  - Cons: Runs synchronously, could slow prompt if expensive

- **Yes (Agent-assisted)**: If detection is expensive or complex
  - Examples: Docker API, Kubernetes API, language version detection
  - Pros: Async, cached, no prompt blocking
  - Cons: More complex implementation

**Decision Matrix**:

| Data Source | Recommended Approach |
|-------------|---------------------|
| Environment variable | Fish-only |
| File existence check | Fish-only |
| Command output < 10ms | Fish-only |
| Command output > 10ms | Agent-assisted |
| External API call | Agent-assisted |
| Complex parsing | Agent-assisted |

**For this example**: We'll implement a Fish-only Docker context segment.

### Phase 2: Create Segment File (30 minutes)

**Location**: `~/.config/fish/gpy/segments/docker.fish`

**Template**:

```fish
# Docker context segment
#
# Shows current Docker context (e.g., "default", "remote-prod")
# Appears only when Docker is available and context is not "default"

function segment_docker_detect
    # Return 0 (success) to show segment, 1 (failure) to hide

    # Check if docker command exists
    command -v docker >/dev/null 2>&1 || return 1

    # Check if Docker is actually running (optional, might be slow)
    # docker info >/dev/null 2>&1 || return 1

    # Check if context is non-default
    set -l context (docker context show 2>/dev/null)
    test -n "$context" -a "$context" != "default"
end

function segment_docker_render
    # Render the segment using GPY helper functions

    # Get current context
    set -l context (docker context show 2>/dev/null)

    # Get color from theme (or use default)
    set -l color $GPY_COLOR_DOCKER
    test -z "$color"; and set color blue

    # Get icon from theme (or use default)
    set -l icon $GPY_ICON_DOCKER
    test -z "$icon"; and set icon "🐳"

    # Render using gpy_section helper
    # Arguments: bg_color fg_color content
    gpy_section_start $color white "$icon $context"
end
```

**Key Functions**:

1. `segment_NAME_detect` - Decide if segment should appear
   - Return 0: Show segment
   - Return 1: Hide segment
   - Should be fast (< 10ms)

2. `segment_NAME_render` - Generate ANSI output
   - Use `gpy_section_start` helper for consistent styling
   - Access theme colors via `$GPY_COLOR_*` variables
   - Access theme icons via `$GPY_ICON_*` variables

**Common Patterns**:

```fish
# Pattern 1: Check command exists
command -v docker >/dev/null 2>&1 || return 1

# Pattern 2: Check file exists
test -f .dockerenv || return 1

# Pattern 3: Check environment variable
test -n "$DOCKER_HOST" || return 1

# Pattern 4: Parse command output
set -l output (some-command 2>/dev/null)
test -n "$output" || return 1
```

### Phase 3: Register Segment (15 minutes)

**Location**: `~/.config/fish/gpy/functions/fish_prompt.fish`

**Find this section** (around line 40-60):

```fish
# Default enabled segments
set -l __enabled_segments \
    directory \
    git \
    language \
    duration \
    status
```

**Add your segment**:

```fish
set -l __enabled_segments \
    directory \
    git \
    language \
    docker \     # ← Add this
    duration \
    status
```

**Order matters**! Segments appear left-to-right in the order listed.

### Phase 4: Add Theme Support (20 minutes)

**Location**: `~/.config/gpy/themes/default.toml`

**Add color and icon**:

```toml
[segments.docker]
enabled = true
bg_color = "#2496ED"  # Docker blue
text_color = "white"

[segments.docker.icon]
icon = "🐳"
nerd_font = ""  # Nerd Font icon if desired
ascii_fallback = "docker"

[segments.docker.open]
icon = ""
bg_color = "match_bg"

[segments.docker.close]
icon = ""
bg_color = "match_bg"
```

**Environment Variable Overrides** (optional):

Users can override in their `config.fish`:

```fish
set -gx GPY_COLOR_DOCKER cyan
set -gx GPY_ICON_DOCKER "🐋"
```

### Phase 5: Test (30 minutes)

**Test Cases**:

1. **Segment appears**:
   ```bash
   docker context use remote-prod
   # → Prompt should show docker segment
   ```

2. **Segment hidden** (default context):
   ```bash
   docker context use default
   # → Segment should disappear
   ```

3. **Segment hidden** (docker not installed):
   ```bash
   # Temporarily hide docker from PATH
   set -l PATH (string match -v '*docker*' $PATH)
   # → Segment should not appear
   ```

4. **Colors and icons**:
   ```bash
   # Verify colors match theme
   # Verify icon appears correctly
   ```

5. **Performance**:
   ```bash
   time fish_prompt
   # Should be < 50ms total for all segments
   ```

**Common Issues**:

| Problem | Cause | Solution |
|---------|-------|----------|
| Segment not appearing | `detect` returning 1 | Add `echo` statements to debug |
| Wrong colors | Theme not loaded | Run `gpy-agent theme export --format fish \| source` |
| Slow prompt | Expensive detection | Move to agent-assisted approach |
| Icons broken | Terminal encoding | Check `echo $GPY_ICON_DOCKER` works |

### Phase 6: Document (15 minutes)

**Update README** (optional but recommended):

```markdown
## Supported Segments

- **Directory**: Current working directory
- **Git**: Branch, status, ahead/behind
- **Language**: Detected language and version
- **Docker**: Current Docker context (NEW!)
- **Duration**: Command execution time
- **Status**: Exit code indicator
```

**Add to [theme-customization.md](../user/theme-customization.md)** (if creating new variables):

```markdown
### Docker Segment

| Variable | Default | Description |
|----------|---------|-------------|
| `GPY_COLOR_DOCKER` | blue | Background color |
| `GPY_ICON_DOCKER` | 🐳 | Docker icon |
```

## Advanced: Agent-Assisted Segment

If your segment needs expensive operations (API calls, complex parsing):

### Additional Steps

**1. Define IPC Message** (`gpy-agent/src/ipc/mod.rs`):

```rust
pub enum Message {
    // ... existing variants
    DockerStatus {
        cwd: PathBuf,
        format: Format,
    },
}
```

**2. Implement Detection** (`gpy-agent/src/docker/detector.rs`):

```rust
pub fn get_docker_status(cwd: &Path) -> Result<DockerInfo> {
    // Call Docker API
    // Parse output
    // Return structured data
}
```

**3. Add to Agent Handler** (`gpy-agent/src/agent.rs`):

```rust
Message::DockerStatus { cwd, format } => {
    let info = docker::get_docker_status(&cwd)?;
    format_response(info, format)
}
```

**4. Update Fish Segment**:

```fish
function segment_docker_render
    # Request data from agent
    set -l response (__gpy_request "docker" $PWD "fish-ansi" false)
    printf '%s' $response
end
```

## Success Criteria

- ✅ Segment appears when expected
- ✅ Segment hidden when not applicable
- ✅ Colors and icons render correctly
- ✅ Prompt renders in < 50ms
- ✅ No errors in `journalctl -u gpy-agent`

## Lessons Learned

### What Works Well

- **Fish-only segments** for fast data (< 10ms)
- **Agent-assisted** for API calls or expensive parsing
- **Clear detect logic** that fails fast
- **Theme integration** for user customization

### Common Mistakes

- ❌ Not checking if command exists (causes errors in logs)
- ❌ Forgetting to add to `__enabled_segments`
- ❌ Expensive operations in `detect` (blocks prompt)
- ❌ Not handling errors (empty output, command failures)

### Optimization Tips

- Cache expensive checks (e.g., Docker API availability)
- Use `command -v` not `which` (faster)
- Redirect stderr to `/dev/null` to avoid error spam
- Consider agent-assistance if detection > 10ms

## Related Tactics

- [Troubleshooting Guide](../user/troubleshooting.md) - When a segment does not render
- [Benchmarking Methodology](performance/benchmarking.md) - When the prompt is slow
- [Advanced: Agent-Assisted Segment](#advanced-agent-assisted-segment) - When detection belongs in the agent

## Metadata

```json
{
  "tactic_id": "add_new_segment",
  "version": "1.2",
  "created": "2024-03-15",
  "last_updated": "2025-01-15",
  "success_count": 15,
  "failure_count": 2,
  "avg_time_minutes": 180,
  "related_tactics": [
    "debug_segment_not_rendering",
    "optimize_slow_prompt",
    "add_agent_detection"
  ],
  "related_concepts": [
    "fish_functions",
    "gpy_helpers",
    "theme_system"
  ],
  "keywords": [
    "segment",
    "prompt",
    "fish",
    "customization",
    "docker"
  ]
}
```

## Changelog

- **v1.2** (2025-01-15): Added Docker example, improved performance guidance
- **v1.1** (2024-09-20): Added agent-assisted section
- **v1.0** (2024-03-15): Initial version
