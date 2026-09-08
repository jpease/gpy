# ADR-0001: Hybrid Rust/Fish Architecture

## Status

Accepted (2024-10)

## Context

Prompt rendering in Fish shell needs to be:
- **Fast**: Sub-millisecond rendering for every command
- **Rich**: Display git status, language detection, custom segments
- **Reliable**: Handle errors gracefully without blocking the shell
- **Cross-platform**: Work on Linux, macOS, and WSL

Pure Fish implementations face several challenges:
- Shell script performance is limited for complex operations (git status, language detection)
- Calling external tools (like `git status`) adds significant latency (100-500ms)
- No background processing capability - everything blocks the prompt
- Limited error handling and recovery mechanisms

Pure Rust implementations also have drawbacks:
- Replacing the entire Fish prompt system is complex
- Fish has excellent prompt customization built-in
- Users expect Fish-native configuration
- Integration with Fish functions and variables is non-trivial

## Decision

Implement a **hybrid architecture** with clear separation of concerns:

### Rust Agent (gpy-agent)
**Responsibilities:**
- Git status detection via a native `git` subprocess (originally gix; see ADR-0003, superseded)
- Language detection with version parsing
- Filesystem watching for live updates
- Caching with TTL management
- IPC server using Unix domain sockets

**Why Rust:**
- Excellent performance for I/O-bound operations
- Strong type system prevents runtime errors
- Great async support via Tokio
- Memory safety without garbage collection
- Cross-compilation for multiple platforms

### Fish Shell Layer
**Responsibilities:**
- Prompt rendering and formatting
- Segment ordering and customization
- Theme application (colors, icons)
- User configuration integration
- IPC client (communicate with agent)

**Why Fish:**
- Native integration with Fish shell features
- Users can customize using familiar Fish scripting
- Direct access to Fish variables and functions
- Existing Fish prompt infrastructure
- No need to reimplement prompt mechanics

### Communication
- **Protocol**: JSON over Unix domain sockets
- **Latency target**: < 1ms for IPC roundtrip
- **Formats**: JSON (structured), fish-ansi (ANSI codes), fish-source (Fish variables)

## Consequences

### Positive

1. **Performance**: Fast Rust operations + lightweight Fish rendering
   - Git status: < 50ms (cached), < 500ms (cold)
   - IPC latency: < 1ms
   - Total prompt render: < 10ms typical

2. **Maintainability**: Clear boundaries between components
   - Rust code handles complex logic
   - Fish code handles presentation
   - Well-defined IPC contract

3. **User Experience**: Best of both worlds
   - Fast, responsive prompts
   - Familiar Fish customization
   - Live updates via filesystem watching

4. **Reliability**: Isolated failure domains
   - Agent crashes don't break the shell
   - Fallback to basic prompt on IPC failure
   - Agent auto-restart capability

### Negative

1. **Complexity**: Two languages to maintain
   - Developers need Rust + Fish knowledge
   - Two testing frameworks (cargo test + fishtape)
   - Build process involves both ecosystems

2. **Deployment**: Additional binary to distribute
   - Must build/ship Rust agent
   - Version compatibility concerns
   - Installation more complex than pure Fish

3. **Debugging**: Cross-language debugging is harder
   - Issues may span Rust/Fish boundary
   - IPC protocol issues require understanding both sides
   - Performance profiling requires multiple tools

### Mitigation Strategies

1. **Clear Documentation**
   - Comprehensive architecture docs
   - IPC protocol specification
   - Contribution guides for both languages

2. **Strong Testing**
   - Unit tests in Rust (cargo test)
   - Integration tests in Fish (fishtape)
   - E2E tests covering IPC interaction
   - Snapshot tests for formatter output

3. **Good Defaults**
   - Agent auto-start on first use
   - Graceful fallback if agent unavailable
   - Sensible default configuration

## Related

- [ADR-0002](adr-0002-unix-domain-sockets-for-ipc.md) - IPC mechanism choice
- [ADR-0003](adr-0003-gix-for-git-operations.md) - Git library selection
- [architecture.md](../architecture.md) - Full system design
