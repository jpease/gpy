# Multi-Shell Architecture Issues & Improvements

> **Archived.** This is historical analysis of shell architecture. Refer to the current shell integration code for active implementations.

**Date**: 2025-11-23
**Status**: Analysis complete, implementation pending

---

## Critical Issues (P0)

### 1. Silent Format Fallback Bug 🐛
**Problem**: `--format zsh` silently returns JSON instead of working or failing loudly.

**This is TWO separate but related bugs:**

**Bug 1A: Unimplemented formats silently fall back to JSON**
```rust
// gpy-agent/src/formatter/mod.rs:198-201
Format::BashSource | Format::Zsh | Format::ZshSource => {
    // Future: implement bash/zsh formatters
    Box::new(JsonFormatter) // ← SILENT FALLBACK!
}
```

**Bug 1B: IPC parser doesn't recognize zsh-* formats**
```rust
// gpy-agent/src/ipc/protocol.rs:341-347
.and_then(|value| match value {
    "json" => Some(Format::Json),
    "fish-ansi" => Some(Format::FishAnsi),
    "fish-source" => Some(Format::FishSource),
    // "zsh-ansi" NOT HERE! Anything else → None
    _ => None,
})
.unwrap_or(Format::Json); // ← Falls back to JSON
```

**User Impact**:
- **CLI**: `gpy-agent oneshot git --format zsh` → returns JSON (not Zsh variables)
- **CLI**: `gpy-agent oneshot git --format zsh-ansi` → returns JSON (not ANSI)
- **IPC**: Zsh requests `{"op":"git","format":"zsh-ansi"}` → parser doesn't recognize → JSON fallback
- **No error message** - completely silent, users assume it's working
- **Interoperability hole** - Zsh can't request Zsh-specific formats even if they existed

**This is a user-facing bug affecting Zsh clients in production.**

**Fix requires addressing BOTH issues:**

1. **Make unimplemented formats fail loudly:**
```rust
Format::Zsh | Format::ZshSource => {
    return Err(Error::config(
        "Format 'zsh' not yet implemented. Use 'fish-ansi' (compatible) or 'json'."
    ));
}
```

2. **Add zsh-ansi to IPC parser (even if aliased):**
```rust
"zsh-ansi" => Some(Format::FishAnsi),  // Alias until formats diverge
"fish-ansi" => Some(Format::FishAnsi),
"fish-source" => Some(Format::FishSource),
```

---

### 2. `FishMessage` Naming Confusion
**Problem**: Struct named "FishMessage" is used by all shells.

```rust
// gpy-agent/src/ipc/protocol.rs:320
/// Fish shell's IPC message format
struct FishMessage {  // <-- Used by Zsh too!
    op: String,
    cwd: Option<String>,
    // ...
}
```

**Impact**: Code is misleading - reviewers think it's Fish-specific when it's not.

**Fix**: Rename to `ShellIpcMessage`:
```rust
/// Shell-agnostic IPC message format
///
/// Used by Fish, Zsh, and other shells that send op-based JSON messages.
/// Distinct from the native Rust Message format.
struct ShellIpcMessage {
    op: String,
    cwd: Option<String>,
    // ...
}
```

**Effort**: ~10 min, single file change.

---

### 3. No Shell Detection
**Problem**: Agent doesn't know which shell is connected.

Current registration:
```json
{"op":"register","pid":12345,"cwd":"/path"}
```

**Missing**: Shell type, version metadata.

**Impact**:
- Can't provide shell-specific optimizations
- Can't debug "it works in Fish but not Zsh" issues
- Can't track adoption metrics by shell

**Fix**: Add optional shell metadata:
```json
{"op":"register","pid":12345,"cwd":"/path","shell":"zsh","shell_version":"5.9"}
```

Make it **optional** for backward compatibility with existing Fish clients.

**Effort**: ~30 min (add fields, update registration handler).

---

## High Priority (P1)

### 4. Format Proliferation
**Problem**: Each shell adds 3+ format variants that often do the same thing.

Current state:
- `FishAnsi` → `FishAnsiFormatter`
- `ZshAnsi` → **maps to same** `FishAnsiFormatter`
- Future: `BashAnsi`, `PowershellAnsi`, etc. (all identical)

**Evidence**:
```rust
// gpy-agent/src/formatter/mod.rs:196
Format::FishAnsi | Format::ZshAnsi => Box::new(FishAnsiFormatter),
```

**Impact**:
- Format enum grows: 3 formats/shell × N shells
- Confusing for users: "Do I use fish-ansi or zsh-ansi?"
- Most map to identical implementations

**Recommendation**: Consolidate to shell-agnostic formats:
- `Ansi` - pre-rendered escape codes (works in any shell)
- `Json` - structured data
- Shell-specific only when truly different

**Migration path** (backward compatible):
1. Add `Ansi` format (maps to current `FishAnsiFormatter`)
2. Alias existing formats: `FishAnsi` → `Ansi`, `ZshAnsi` → `Ansi`
3. Deprecate shell-prefixed variants
4. Remove in next major version

**Effort**: ~2 hours.

---

### 5. Theme Export Duplication
**Problem**: Separate export method per shell that just changes syntax.

```rust
// gpy-agent/src/theme/manager.rs
pub fn export_fish(&self, config: &Config) -> String {
    // Output: set -g VAR "value"
}

pub fn export_zsh(&self, config: &Config) -> String {
    // Output: typeset -g VAR="value"
}

// Future: export_bash(), export_powershell(), ...
```

**Impact**:
- ThemeManager grows by 1 method per shell
- Core logic duplicated across methods
- Bug fixes require updating N functions

**Recommendation**: Template-based approach:
```rust
pub enum Shell { Fish, Zsh, Bash }

impl Shell {
    fn variable_syntax(&self) -> (&str, &str) {
        match self {
            Fish => ("set -g ", ""),
            Zsh => ("typeset -g ", "="),
            Bash => ("export ", "="),
        }
    }
}

pub fn export(&self, shell: Shell, config: &Config) -> String {
    let (prefix, sep) = shell.variable_syntax();
    // Single implementation for all shells
}
```

**Effort**: ~3 hours.

---

### 6. Testing Fragmentation
**Problem**: No cross-shell consistency tests.

Current:
- `tests/fish/` - Fish-specific tests
- `tests/zsh/` - Zsh-specific tests
- **No verification** that they behave identically

**Impact**: Shells can drift apart, inconsistent UX.

**Example test**:
```bash
# tests/cross-shell/prompt_parity.sh
fish_output=$(fish -c 'cd /tmp/repo && source gpy && fish_prompt')
zsh_output=$(zsh -c 'cd /tmp/repo && source gpy.zsh && __gpy_prompt')

# Strip ANSI escape codes
fish_stripped=$(echo "$fish_output" | sed 's/\x1b\[[0-9;]*m//g')
zsh_stripped=$(echo "$zsh_output" | sed 's/\x1b\[[0-9;]*m//g')

# Compare structure (git branch, directory, etc.)
if [[ "$fish_stripped" != "$zsh_stripped" ]]; then
    echo "FAIL: Prompts differ!"
    diff <(echo "$fish_stripped") <(echo "$zsh_stripped")
    exit 1
fi
```

**Effort**: ~1 hour for initial test, expand over time.

---

## Medium Priority (P2)

### 7. Socket Path Logic Duplication
**Problem**: XDG precedence logic duplicated in each shell.

**Fish** (core/ipc.fish):
```fish
if set -q XDG_RUNTIME_DIR
    echo "$XDG_RUNTIME_DIR/gpy/gpy.sock"
else if set -q XDG_CACHE_HOME
    echo "$XDG_CACHE_HOME/gpy/gpy.sock"
else
    echo "$HOME/.cache/gpy/gpy.sock"
fi
```

**Zsh** (core/ipc.zsh) - **identical logic, different syntax**:
```zsh
if [[ -n "$XDG_RUNTIME_DIR" ]]; then
    echo "$XDG_RUNTIME_DIR/gpy/gpy.sock"
elif [[ -n "$XDG_CACHE_HOME" ]]; then
    echo "$XDG_CACHE_HOME/gpy/gpy.sock"
else
    echo "$HOME/.cache/gpy/gpy.sock"
fi
```

**Impact**: Changes (e.g., add `GPY_SOCKET_PATH` override) require updating all shells.

**Recommendation**:
- Document canonical algorithm in spec
- Add test that verifies all shells use same logic
- Long-term: `gpy-agent socket-path` command shells can call

---

### 8. Segment Rendering Boundary Unclear
**Current split**:
- **Agent renders**: git, language (pre-formatted ANSI via IPC)
- **Shell renders**: directory, clock, duration (dynamic per-prompt)

**Problem**: Split isn't documented anywhere.

**Hypothesis** (needs verification):
- Agent renders data requiring caching/watching (git, lang detection)
- Shell renders data that changes per-prompt (time, directory)

**Recommendation**: Document in architecture docs.

---

### 9. Configuration Source of Truth
**Problem**: Defaults scattered across agent and shells.

- **Agent**: Theme TOML has `enabled_segments = ["clock", "duration", ...]`
- **Fish**: Hardcoded `set -g __enabled_segments clock duration language directory git`
- **Zsh**: Phase 1 has `(directory git)`, will become `(clock duration ...)`

**Impact**: Shells can have different defaults, confusing UX.

**Recommendation**: Agent is authoritative:
1. Agent theme export includes `__enabled_segments`
2. Shells use that or their own override
3. Remove hardcoded defaults from shell files

---

## Implementation Plan

### Phase A: Fix Silent Bugs (P0, ~1 hour)
**Before Phase 2 of Zsh support**

1. **Add format validation** (~20 min)
   - Make unimplemented formats fail loudly
   - Add `zsh-ansi` to IPC parser (alias to `fish-ansi`)

2. **Rename `FishMessage`** (~10 min)
   - Rename struct to `ShellIpcMessage`
   - Update comments

3. **Add shell detection** (~30 min)
   - Add optional `shell`/`shell_version` fields to register
   - Log which shells are connecting
   - Update Zsh to send shell metadata

**Deliverable**: No silent failures, better observability.

---

### Phase B: Reduce Duplication (P1, ~6 hours)
**During Zsh Phase 2 or before Bash**

4. **Consolidate format enums** (~2 hours)
   - Add `Ansi` format
   - Alias shell-specific formats
   - Update CLI help text

5. **Template-based theme export** (~3 hours)
   - Single `export(shell)` method
   - Remove `export_fish()`, `export_zsh()`

6. **Cross-shell parity test** (~1 hour)
   - Compare Fish vs Zsh output
   - Run in CI

**Deliverable**: Cleaner codebase, regression protection.

---

### Phase C: Documentation & Polish (P2, ~2 hours)
**After Zsh Phase 2**

7. **Document segment boundary** (~30 min)
8. **Document socket path algorithm** (~30 min)
9. **Centralize config defaults** (~1 hour)

**Deliverable**: Clear architecture, easier onboarding.

---

## Backward Compatibility Strategy

**Critical**: Don't break existing Fish users.

### Safe Changes:
- ✅ Rename internal structs (`FishMessage` → `ShellIpcMessage`)
- ✅ Add optional fields to IPC (`"shell"` in register)
- ✅ Add new format variants (`Ansi`) while keeping old ones

### Requires Migration:
- ⚠️ Removing format variants (deprecate first, remove in major version)
- ⚠️ Changing theme export API (keep both methods during transition)

### Testing Strategy:
1. All existing Fish tests must pass
2. Add backward compatibility tests (old-style IPC messages)
3. Cross-shell tests catch regressions

---

## Prioritization Matrix

| Issue | User Impact | Effort | Priority |
|-------|-------------|--------|----------|
| 1. Silent format fallback | **High** (bug) | Low | **P0** |
| 2. FishMessage naming | Low (internal) | Low | **P0** (easy win) |
| 3. Shell detection | Medium (debugging) | Low | **P0** (foundation) |
| 4. Format proliferation | Medium (confusion) | Medium | **P1** |
| 5. Theme export duplication | Low (maintainability) | Medium | **P1** |
| 6. Testing fragmentation | **High** (regressions) | Medium | **P1** |
| 7. Socket path duplication | Low (rare changes) | Low | P2 |
| 8. Segment boundary docs | Low (documentation) | Low | P2 |
| 9. Config source of truth | Low (edge cases) | High | P2 |

---

## Success Metrics

After Phase A:
- ✅ No silent format failures
- ✅ Agent logs shell type for each connection
- ✅ Code is self-documenting (`ShellIpcMessage`)

After Phase B:
- ✅ Format count stops growing linearly with shells
- ✅ Theme export is single method
- ✅ Cross-shell tests catch 90% of regressions

After Phase C:
- ✅ New contributors understand architecture
- ✅ Adding new shell takes <1 day (vs current ~4 days)

---

## Recommendation

**Do Phase A (P0 items) now** - ~1 hour of work prevents:
- User confusion (silent failures)
- Future debugging pain (no shell metadata)
- Code confusion (misleading names)

Then proceed with Zsh Phase 2 on solid foundation.
