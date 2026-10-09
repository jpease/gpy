# GPY Design Decisions

## 🎯 Core Principle: Solve Real Problems for Real Users

GPY is a **Fish shell prompt enhancement tool**. All design decisions should be evaluated against:

1. **Does this solve a problem Fish shell users actually have?**
2. **Does this improve the user experience meaningfully?**
3. **Is the complexity justified by the benefit?**

## 🚫 Anti-Patterns to Avoid

### ❌ **Over-Engineering for Theoretical Platforms**

**Problem**: Building cross-platform support for platforms where Fish shell doesn't run.

**Example**: The original transport layer supported Windows named pipes and TCP fallback.

**Reality Check**:
- Fish shell is Unix-only
- Windows users run Fish in WSL
- WSL supports Unix domain sockets perfectly
- TCP fallback would never be used in practice

**Decision**: Use **Unix domain sockets primarily**. Simple, clean, covers 100% of real usage on Unix-like systems.
*Note: There is no Windows transport. Named pipes are not implemented: `ipc/transport.rs` and the client/server entry points return `native_windows_unsupported()` on non-Unix targets, so native Windows is CLI-only and the prompt integration runs under WSL.*

### ❌ **Premature Optimization**

**Problem**: Adding complexity for performance gains not proven to be needed.

**Example**: Complex caching layers, connection pooling, etc.

**Reality Check**:
- Fish prompt needs to render in <50ms total
- IPC should be <1ms
- Simple Unix socket is already sub-millisecond
- Git command execution is the real bottleneck, not IPC

**Decision**: Profile first, optimize second.

### ❌ **Enterprise Features for Personal Tools**

**Problem**: Adding enterprise-grade features for a personal productivity tool.

**Example**: Complex authentication, audit logging, distributed deployment, etc.

**Reality Check**:
- GPY runs locally on developer machines
- Single user per agent instance
- No network communication beyond local sockets
- No compliance requirements

**Decision**: Keep it simple and focused on developer productivity.

## ✅ Justified Complexity

### ✅ **TOML Configuration**

**Why Complex**: Users need to customize timeouts, themes, language preferences.

**User Benefit**: Essential for usability - different users have different needs.

**Complexity Level**: Moderate - serde integration, validation, defaults.

**Verdict**: ✅ **Justified** - directly improves UX.

### ✅ **Comprehensive Error Handling**

**Why Complex**: Custom Error enum, proper Result propagation, descriptive messages.

**User Benefit**: Reliability - prompt never hangs, clear error messages.

**Complexity Level**: Moderate - thiserror, proper error chains.

**Verdict**: ✅ **Justified** - essential for production reliability.

### ✅ **Async/Await Architecture**

**Why Complex**: Tokio runtime, async traits, futures.

**User Benefit**: Non-blocking prompt rendering, concurrent operations.

**Complexity Level**: High - but necessary for performance.

**Verdict**: ✅ **Justified** - required for <50ms latency target.

### ✅ **Optimized Git Subprocess (Native Git)**

**Why Complex**: Manually parsing `git status --porcelain=v2` output.

**User Benefit**: **High performance & Stability** - leveraging Git's own optimizations (untracked cache, fsmonitor).

**Complexity Level**: Moderate - output parsing, version compatibility.

**Decision Rationale**:
- `gix` (Gitoxide) was promising but lacked full feature parity (e.g., sparse checkouts, submodules)
- `libgit2` introduced C dependency complexity
- Native `git` command has matured significantly (v2 porcelain format)
- Leveraging `core.untrackedCache` allows `git status` to be extremely fast (<40ms)

**Verdict**: ✅ **Justified** - reliable, standard, and fast enough with optimizations.

### ✅ **Native Language Detection (gengo-language)**

**Why Complex**: Replace file extension checking with linguist-grade language detection.

**User Benefit**: **Accurate multi-language projects** without per-language configuration.

**Complexity Level**: Low for the matching; GPY owns the directory walk.

**Decision Rationale**:
- Simple file extension detection missed complex projects (monorepos, mixed languages)
- `gengo-language` supplies linguist's matcher tables (filenames, path globs,
  extensions, interpreters, content heuristics) as a data dependency
- The walk stays GPY's own: the `gengo` wrapper crate's `Gengo::analyze` reads
  every file in the tree (546 ms on 20k files against a 200 ms hard max) and
  its default features pull `gix` back in, the git backend retired in 0efbaa24
- Detection is path-first — a filename, glob or extension that names exactly
  one language never opens the file — so only ambiguous paths pay for a read

**History**: `hyperpolyglot` filled this role until #523 (epic #519), which
replaced it with a maintained dependency. The swap preserved its behaviour
deliberately: the same 51,200-byte read window, the same
extension-before-shebang precedence, and the same linguist vendor and
documentation exclusion globs, now vendored in
`gpy-agent/src/language/filters.rs`.

**Verdict**: ✅ **Justified** - better accuracy with a maintained dependency and
a walk GPY can bound itself.

### ✅ **Simple Background Process**

**Why Chosen Over Full Daemonization**: Cross-platform compatibility with Fish shell.

**User Benefit**: Live prompt updates when files change (instant visual feedback).

**Complexity Level**: Low - basic process forking, no complex daemon management.

**Decision Rationale**:
- Fish runs on Linux, macOS, BSD, Unix - needs universal solution
- System-specific daemon managers (systemd, launchd) break portability
- `daemonize` crate is unmaintained and has Tokio conflicts
- Simple fork() + setsid() works everywhere Fish works

**Alternative Considered**: `daemonizr` crate - viable if full daemon features needed.

**Verdict**: ✅ **Justified** - matches Fish's cross-platform philosophy.

## 🎯 Decision Framework

Before adding complexity, ask:

1. **User Problem**: What specific Fish shell user problem does this solve?
2. **Usage Data**: How many users will actually use this feature?
3. **Platform Reality**: Does Fish shell even run on the platforms this targets?
4. **Simpler Alternative**: Is there a 20% solution that gives 80% of the benefit?
5. **Maintenance Cost**: Who will maintain this code when edge cases arise?

## 📊 Current Complexity Budget

**High Complexity (Justified)**:
- Async/await architecture
- **Optimized git subprocesses** - robust parsing of porcelain v2
- **Native language detection (gengo-language)** - linguist-grade accuracy over a GPY-owned walk

**Medium Complexity (Justified)**:
- TOML configuration system
- Error handling framework
- File watching/debouncing

**Low Complexity (Keep Simple)**:
- IPC transport (Unix sockets only)
- Protocol serialization (JSON)
- Path handling (XDG + fallback)
- **Background process management** - simple fork()/setsid() for cross-platform compatibility

## 🔄 Review Process

When adding features:

1. **Write the user story first**: "As a Fish user, I want X so that Y"
2. **Prove the problem exists**: Evidence that users actually need this
3. **Start with the simplest solution**: Can this be solved in 10 lines instead of 100?
4. **Measure the impact**: Profile performance, measure complexity increase
5. **Document the decision**: Update this file with rationale

## 🎯 Success Metrics

**Primary**:
- Prompt rendering <50ms (user-visible performance)
- Zero crashes/hangs (reliability)
- Easy installation (Fish community adoption)

**Secondary**:
- Clean, maintainable code (developer experience)
- Comprehensive test coverage (confidence in changes)
- Clear error messages (debugging experience)

**Not Success Metrics**:
- Platform coverage beyond where Fish runs
- Enterprise-grade features
- Complex architectural patterns for their own sake

---

## 🔄 Lessons Learned

### ✅ **Idiomatic Patterns Over Custom Solutions**

**Problem**: Custom nested control flow patterns reduce maintainability.

**Solution**: Use proven language idioms - Rust combinators, Fish argument parsing.

**Example**: Replaced nested `if` statements with `filter_map()`, `then_some()`, early returns.

**Result**: Cleaner, more maintainable code following community standards.

### ✅ **Proven Libraries Over Custom Implementations**

**Problem**: Custom socket handling, daemon management creates maintenance burden.

**Solution**: Use battle-tested libraries - `notify` for file watching, avoid unmaintained crates.

**Research Insight**: Always check maintenance status - `daemonize` was unmaintained with security advisories.

**Result**: More reliable foundation, less custom code to debug.

### ✅ **Cross-Platform First**

**Problem**: Platform-specific solutions break Fish's portability promise.

**Solution**: Design for Fish's supported platforms from day one.

**Example**: Simple background processes work everywhere Fish works vs system-specific daemon managers.

**Result**: Single solution that works universally.

---

**Remember**: GPY should be the **best possible Fish shell enhancement**, not a showcase of software architecture patterns.
