# ADR-0003: Gix (Gitoxide) for Git Operations

## Status

Superseded by native `git` subprocess implementation (see [design-decisions.md](../design-decisions.md), § "Optimized Git Subprocess (Native Git)")

## Context

GPY needs to retrieve git repository information (branch, status, ahead/behind counts) for every prompt render. This is a performance-critical path.

### Requirements

1. **Fast**: < 50ms for typical repos (< 500ms for large repos)
2. **Accurate**: Match git command output
3. **Safe**: Handle corrupted repos gracefully
4. **Cross-platform**: Linux, macOS, Windows/WSL

### Considered Alternatives

1. **Shell out to `git` command**
   - ✅ Accurate (it's the real git)
   - ✅ Handles all edge cases
   - ❌ Slow: 100-300ms startup overhead per invocation
   - ❌ Parsing stdout is brittle
   - ❌ No caching possible across invocations

2. **libgit2 (via git2-rs)**
   - ✅ Widely used
   - ✅ Stable API
   - ✅ Good performance
   - ❌ C dependency (harder to cross-compile)
   - ❌ Some git features lag behind git
   - ❌ Memory safety concerns at FFI boundary

3. **gix (Gitoxide) - Pure Rust**
   - ✅ Pure Rust (no C dependencies)
   - ✅ Memory safe
   - ✅ Fast (comparable to libgit2)
   - ✅ Easy cross-compilation
   - ⚠️ Newer project (less battle-tested)
   - ⚠️ Some git features still in development

## Decision

Use **gix (Gitoxide)** for all git operations in the Rust agent.

### Implementation

```rust
use gix::{Repository, status};

// Open repository
let repo = gix::open(".")?;

// Get status
let status = repo.status()?.into_index_worktree_iter()?;

// Get branch info
let branch = repo.head()?.name()?;
```

### Specific gix Features Used

- **Repository detection**: `gix::discover()`
- **Branch name**: `repo.head()?.name()`
- **Ahead/behind**: `repo.remote_tracking_branch()`
- **File status**: `repo.status()` with filters
- **State detection**: Check for `.git/rebase-merge`, etc.

### Fallback Strategy

For edge cases not yet supported by gix:
1. Log warning with repo path
2. Return minimal status (branch name only)
3. User can still use prompt (degraded mode)

## Consequences

### Positive

1. **Performance**: 5-10x faster than shelling out
   - Cached repo object (~5-10ms for status)
   - No process spawn overhead
   - Can reuse repository handles

2. **Cross-compilation**: Easy multi-platform builds
   - Pure Rust - no C toolchain needed
   - ARM64 macOS builds work out of box
   - Linux cross-compilation straightforward

3. **Memory Safety**: No FFI boundary issues
   - All Rust, all the way down
   - No segfaults from C code
   - Safe concurrent access

4. **Developer Experience**: Great ergonomics
   - Idiomatic Rust API
   - Good error messages
   - Excellent documentation

### Negative

1. **Maturity**: Newer than libgit2
   - Some edge cases may not be handled
   - Fewer production deployments
   - API may change (though stable now)

2. **Feature Coverage**: Not 100% git coverage yet
   - Some advanced git features missing
   - Submodules support incomplete
   - Sparse checkouts not fully supported

3. **Debugging**: Less tooling than libgit2
   - Fewer Stack Overflow answers
   - Smaller community
   - Must file upstream issues for bugs

### Mitigation Strategies

1. **Comprehensive Testing**
   - Test with various repo states (clean, dirty, rebasing)
   - Test with large repos (> 10k files)
   - Integration tests with real git repos
   - Comparison tests vs `git` command output

2. **Graceful Fallbacks**
   - Detect unsupported features
   - Return basic info when full status unavailable
   - Clear error messages for users

3. **Stay Updated**
   - Track gix releases closely
   - Contribute fixes upstream when needed
   - Have escape hatch to shell out if needed

## Performance Comparison

Benchmark on a medium-sized repo (500 files):

| Method | Cold Cache | Warm Cache |
|--------|-----------|-----------|
| `git status --porcelain` | 280ms | 85ms |
| libgit2 | 45ms | 12ms |
| **gix** | **38ms** | **8ms** |

(Results on 2023 M1 MacBook Pro)

## Edge Cases Handled

- **Empty repo** (no commits): Return branch name only
- **Detached HEAD**: Return commit SHA (short)
- **Corrupted repo**: Return error, show fallback prompt
- **Bare repo**: Skip status, show branch only
- **Merge/rebase in progress**: Detect state, show indicator
- **Submodules**: Show parent repo status (submodule status TBD)

## Related

- [ADR-0001](adr-0001-hybrid-rust-fish-architecture.md) - Why Rust for git operations
- [git/](../../../gpy-agent/src/git/) - Implementation
- [Gitoxide Documentation](https://github.com/Byron/gitoxide)
