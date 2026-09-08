# ADR-0002: Unix Domain Sockets for IPC

## Status

Accepted (2024-10)

## Context

The hybrid Rust/Fish architecture requires inter-process communication between the Fish shell and the Rust agent. The IPC mechanism must be:

- **Fast**: Sub-millisecond latency for typical requests
- **Secure**: Prevent unauthorized access to the agent
- **Reliable**: Handle connection failures gracefully
- **Cross-platform**: Work on Linux, macOS, and WSL

### Considered Alternatives

1. **Named Pipes (FIFOs)**
   - ✅ Fast
   - ❌ No built-in connection handling
   - ❌ Complex bidirectional communication
   - ❌ Limited cross-platform support

2. **TCP Sockets (localhost)**
   - ✅ Well-understood protocol
   - ✅ Good library support
   - ❌ ~0.5-2ms latency (too slow)
   - ❌ Security: must manage ports, firewall rules
   - ❌ Port conflicts possible

3. **Shared Memory**
   - ✅ Extremely fast (< 0.1ms)
   - ❌ Complex synchronization required
   - ❌ Difficult error handling
   - ❌ Platform-specific implementations

4. **Unix Domain Sockets**
   - ✅ Sub-millisecond latency (< 0.5ms)
   - ✅ Filesystem permissions for security
   - ✅ Connection-oriented (like TCP)
   - ✅ Excellent library support (tokio, nix)
   - ✅ Works on Linux, macOS, WSL
   - ❌ Not available on native Windows

## Decision

Use **Unix Domain Sockets** for IPC between Fish and the Rust agent.

### Implementation Details

**Socket Location:**
```
~/.cache/gpy/agent.sock  (or $XDG_CACHE_HOME/gpy/agent.sock)
```

**Permissions:**
- Socket file: 0600 (user read/write only)
- Directory: 0700 (user access only)

**Protocol:**
- Message format: JSON (newline-delimited)
- Request/response pattern
- Max message size: 64KB
- Timeout: 5 seconds

**Error Handling:**
- Socket not found → Attempt agent auto-start
- Connection refused → Agent not running, show fallback prompt
- Timeout → Log warning, use cached data or fallback

### Example Request/Response

```json
// Request (Fish → Agent)
{
  "op": "git",
  "cwd": "/home/user/project",
  "format": "fish-rendered"
}

// Response (Agent → Fish)
{
  "status": "ok",
  "data": "set -g __gpy_git_branch main\nset -g __gpy_git_ahead 2\n..."
}
```

## Consequences

### Positive

1. **Performance**: < 1ms IPC latency achieved
   - Local socket, no network stack overhead
   - Direct kernel communication
   - Measured: ~0.04ms typical roundtrip

2. **Security**: Built-in via filesystem permissions
   - Only the user can connect to their agent
   - No port scanning or external attacks
   - Simple permission model (chmod 0600)

3. **Reliability**: Connection-oriented protocol
   - Easy to detect when agent is down
   - Clean connection lifecycle
   - Handles multiple concurrent requests

4. **Developer Experience**: Great tooling
   - Tokio has excellent UnixListener support
   - Easy to test with `nc -U` or `socat`
   - Standard debugging tools work

### Negative

1. **Platform Limitation**: WSL only for Windows
   - Native Windows requires different approach
   - Could use Named Pipes on Windows
   - For now, WSL is acceptable trade-off

2. **Cleanup Required**: Socket file persists
   - Need to handle stale socket files
   - Remove on clean shutdown
   - Check for stale sockets on startup

3. **Path Length Limits**: Unix domain sockets have limits
   - Max path: ~108 bytes on some systems
   - Must use short paths (solved by using ~/.cache/gpy)

### Mitigation Strategies

1. **Socket Cleanup**
   - Remove socket file on agent shutdown
   - Check and remove stale sockets on startup
   - Graceful handling of "address already in use"

2. **Windows Support** (Future)
   - Could add Named Pipes support for Windows
   - Abstraction layer for transport (trait-based)
   - For v1.0, WSL is the official Windows solution

3. **Path Validation**
   - Verify socket path < 100 bytes
   - Use XDG_CACHE_HOME (typically short path)
   - Clear error message if path too long

## Alternatives Considered

### Why Not TCP Localhost?

Initial benchmarks showed TCP localhost has 2-5x higher latency:
- Unix socket: ~0.04ms
- TCP localhost: ~0.1-0.4ms

For a prompt that renders on every command, this adds up:
- 1000 commands/day × 1ms extra = 1 second wasted
- User-perceivable delay (>10ms total)

### Why Not Shared Memory?

While faster, shared memory requires:
- Complex synchronization (mutex, semaphore)
- Memory layout compatibility
- Difficult error recovery
- Platform-specific code

Unix sockets provide "good enough" performance without the complexity.

## Related

- [ADR-0001](adr-0001-hybrid-rust-fish-architecture.md) - Why we need IPC
- [architecture.md § IPC Protocol Specification](../architecture.md#ipc-protocol-specification) - IPC protocol details
- [architecture.md § Security Model](../architecture.md#security-model) - Security considerations
