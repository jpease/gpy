# Bash Support: Limitations and Caveats

**Last Updated**: 2025-11-26

## Overview

GPY supports Bash with **functional parity** to Fish and Zsh for core features (git status, language detection, segments). However, Bash has inherent limitations that affect some advanced features.

This document clearly outlines what works, what has caveats, and what's unavailable in Bash.

---

## ✅ Fully Supported Features

These features work identically to Fish/Zsh:

- **Git status** - Branch, ahead/behind, staged/unstaged counts, states
- **Language detection** - All 10 languages with version detection
- **Directory segment** - Current directory with abbreviation
- **Status segment** - Exit code indicator (✔/✖)
- **Clock segment** - Current time
- **Agent IPC** - Full daemon communication
- **Theme system** - Complete color/icon customization
- **Config hot-reload** - Config changes apply at the next prompt (no SIGURG handler; see the Live updates notes in the matrix)
- **Caching** - Sub-millisecond cache hits

---

## ⚠️ Limited/Degraded Features

### 1. Command Duration Tracking

**Limitation**: Precision depends on Bash version

| Bash Version | Duration Precision | Notes |
|--------------|-------------------|-------|
| **Bash 5.0+** | ✅ Millisecond | Uses `EPOCHREALTIME` (like Zsh) |
| **Bash 4.x** | ⚠️ 10-20ms overhead | Uses `date +%s%N` (slower) |
| **Bash 3.x** | ❌ Not supported | No nanosecond timing, segment disabled |

**Impact**:
- Bash 5: Works perfectly, matches Zsh
- Bash 4: Works but adds ~15ms overhead per prompt render
- Bash 3: Duration segment will not display (macOS default)

**Workaround**: Upgrade to Bash 5 via Homebrew (macOS) or package manager (Linux)

```bash
# macOS: Install Bash 5
brew install bash
sudo bash -c 'echo /opt/homebrew/bin/bash >> /etc/shells'
chsh -s /opt/homebrew/bin/bash
```

---

### 2. Live Updates

**Limitation**: An idle Bash prompt does not repaint by itself.

**What is measured** (`tests/bash/e2e_git_live_content.test.bash`, a real
`bash -i` on a pseudo-terminal against a real agent): when a tracked file
changes while the shell sits at an idle prompt, the agent updates its cache
and rings SIGURG, but readline has already drawn the previous prompt and has
no `reset-prompt`, so the screen does not change until the next prompt. Press
Enter (or run any command) and the new state is there. Fish and Zsh repaint
the idle prompt in place; see
[Troubleshooting → Shell Comparison](troubleshooting.md#shell-comparison-at-a-glance).

Bash installs no SIGURG trap (#678). A trap could only re-render a `PS1`
readline cannot show: on Bash 5 a doorbell arriving during a render started a
nested one, so a steady stream of doorbells hung the shell, Bash 3.2 ran the
trap at an idle prompt and re-rendered for nothing, and every doorbell made a
running `wait` return early. SIGURG keeps its default disposition, ignore, so
it never interrupts a command or `wait` and never kills a shell. The agent's
`<pid>.reload` and `<pid>.reregister` flag files are read when the next prompt
is drawn instead.

**Mitigation**: Every prompt render reads the agent's current state, so the
change is never lost — it is one Enter away. The agent keeps its cache current
as files change, so that next prompt is instant.

Bash re-registers after an agent restart at its next prompt (#638), and
applies a config reload the same way: the agent leaves a `<pid>.reregister`
or `<pid>.reload` flag, and the prompt drawn after the next Enter consumes it
before rendering. A dead agent is recovered by the periodic supervisor check,
so live updates resume without reopening the shell.

---

### 3. Performance

**Limitation**: ~20-30% slower than Zsh, ~50% slower than Fish

**Why**:
- Bash string operations are slower
- Less optimized array handling
- `PROMPT_COMMAND` overhead
- No built-in JSON escaping

**Numbers**: the render-time budgets every gate enforces are in
`tests/performance-baselines.json` (`./scripts/bench.sh --ci` checks them;
`./scripts/bench.sh` measures your machine with hyperfine). The per-shell
table that used to sit here was measured on 2025-11-26, before the render
loop and cache changes of #341-#343 and #614, and is not reproduced; run the
benchmark script for a current figure.

---

### 4. Prompt Customization

**Limitation**: Bash's `PS1` uses different escape sequences

**Differences**:

| Feature | Zsh | Bash |
|---------|-----|------|
| Username | `%n` | `\u` |
| Hostname | `%m` | `\h` |
| Directory | `%~` | `\w` |
| Time | `%D{%H:%M}` | `\t` or `\A` |
| Colors | `%F{color}` | `\[\033[...m\]` |

**Impact**: GPY handles this internally, but custom `PS1` tweaks differ between shells

---

## ❌ Unavailable Features

### 1. Transient Prompt

**Status**: Not implemented in any shell (Fish, Zsh or Bash)

**Why**:
- No shell integration collapses the previous prompt after you press Enter
- In Bash it would additionally need fragile terminal control sequences to rewrite prompt history, with a high risk of breaking terminal state

**Workaround**: Use full prompt mode (standard behavior)

---

### 2. Sub-millisecond Cache Display

**Status**: Bash 3.x only (macOS default)

**Why**: No nanosecond timing available

**Impact**: Can't display "0.1ms" cache hit times (shows as "1ms" minimum)

---

## 🔧 Version Recommendations

### Minimum Requirements

- **Bash 3.2+** required for basic functionality (macOS default)
- **Bash 5.0+** recommended for full experience

### Optimal Setup

- **Bash 5.3+** for best performance and features
- Modern terminal emulator (iTerm2, Alacritty, WezTerm)
- Nerd Font for icons

### Version Detection

GPY automatically detects Bash version and disables incompatible features:

```bash
# Check your Bash version
bash --version

# Example output:
# GNU bash, version 5.3.8(1)-release  ← Good, full features
# GNU bash, version 3.2.57(1)-release ← macOS default, limited
```

---

## 📊 Feature Matrix

| Feature | Fish | Zsh | Bash 5 | Bash 4 | Bash 3 |
|---------|------|-----|--------|--------|--------|
| Git status | ✅ | ✅ | ✅ | ✅ | ✅ |
| Language detection | ✅ | ✅ | ✅ | ✅ | ✅ |
| Status indicator | ✅ | ✅ | ✅ | ✅ | ✅ |
| Clock | ✅ | ✅ | ✅ | ✅ | ✅ |
| Directory | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Duration** | ✅ | ✅ | ✅ | ⚠️ | ❌ |
| **Live updates** | ✅ idle prompt repaints | ✅ idle prompt repaints | ⚠️ shown at next prompt | ⚠️ shown at next prompt | ⚠️ shown at next prompt |
| Config hot-reload | ✅ | ✅ | ✅ | ✅ | ✅ |
| Agent IPC | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Performance** | A+ | A | B+ | B | B- |
| Transient prompt | ❌ | ❌ | ❌ | ❌ | ❌ |
| Exit cleanup | ✅ | ✅ | ✅ | ✅ | ✅ |

**Legend**: ✅ Full support | ⚠️ Works with caveats | ❌ Not available

---

## 🎯 Recommendations

### For Best Experience

1. **Upgrade to Bash 5** if possible (especially macOS users)
2. Use a modern terminal emulator
3. Install a Nerd Font
4. Consider Zsh if you need all features

### When to Use Bash GPY

✅ **Good fit**:
- Bash is required (CI/CD, enterprise environments)
- You're already using Bash 5
- You want cross-shell consistency
- Performance is "good enough" (6-8ms)

⚠️ **Consider alternatives**:
- macOS with default Bash 3.2 → Use Fish or Zsh
- Need absolute fastest performance → Use Fish or Zsh

### Migration Path

If you're on Bash 3.2 (macOS):

```bash
# Option 1: Upgrade Bash (keeps Bash familiarity)
brew install bash
chsh -s /opt/homebrew/bin/bash

# Option 2: Switch to Zsh (native macOS, full features)
chsh -s /bin/zsh

# Option 3: Switch to Fish (fastest, best experience)
brew install fish
chsh -s /opt/homebrew/bin/fish
```

---

## 🐛 Known Issues

### Issue: Duration shows "0ms" on first prompt
**Bash version**: All
**Cause**: `PROMPT_COMMAND` hasn't run yet
**Workaround**: Displays correctly after first command
**Severity**: Cosmetic

### Issue: Agent updates during a command show at the next prompt
**Bash version**: All
**Cause**: Bash ignores the agent's SIGURG doorbell (no trap, #678); its flags are read when the next prompt is drawn
**Workaround**: None needed: the prompt after the command shows the current state
**Severity**: Minor (readline cannot repaint a prompt in place)

### Issue: Prompt renders slowly on Bash 3.2
**Bash version**: 3.x
**Cause**: Slow string operations + `date` overhead
**Workaround**: Upgrade to Bash 5 or switch shells
**Severity**: Moderate

---

## 📝 Summary

Bash support in GPY is **production-ready** with understood trade-offs:

- ✅ **Core functionality**: 100% feature parity for git/language/segments
- ⚠️ **Advanced features**: Some limitations (duration precision, live updates reliability)
- ⚠️ **Performance**: 20-30% slower than Zsh, still faster than Starship
- ❌ **Missing**: Transient prompt (not implemented in any shell)

**Bottom line**: Bash 5 users get ~90% of the Fish/Zsh experience. Bash 3/4 users get ~75%. Still better than most alternatives.
