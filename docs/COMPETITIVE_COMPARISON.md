# Competitive Comparison Matrix

**Last Updated:** 2026-05-15
**Version:** 1.0.0

This document provides a comprehensive comparison of GPY against other leading shell prompt solutions based on documented features, benchmarks, and architectural analysis.

---

## Quick Summary

| Prompt | Performance | Language Detection | Architecture | Best For |
|--------|-------------|-------------------|--------------|----------|
| **GPY** | ⭐⭐⭐⭐⭐ Sub-40ms | ⭐⭐⭐⭐⭐ Confidence-based | Hybrid Rust+Shell | Polyglot repos, Fish/Zsh users |
| **Starship** | ⭐⭐⭐⭐ ~40ms | ⭐⭐⭐⭐ Present-based | Pure Rust | Cross-shell consistency |
| **Powerlevel10k** | ⭐⭐⭐⭐⭐ Very fast | ⭐⭐⭐ Version managers | Pure Zsh | Zsh power users |
| **Oh-My-Posh** | ⭐⭐⭐ Good | ⭐⭐⭐ Present-based | Go | Windows/PowerShell users |
| **Pure** | ⭐⭐⭐⭐⭐ Minimal | ⭐ Basic | Pure Zsh | Minimalists |
| **Tide** | ⭐⭐⭐⭐ Good | ⭐⭐⭐ Basic | Pure Fish | Fish-only users |

---

## Detailed Feature Comparison

### Core Features

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Shell Support** | | | | | | |
| Fish | ✅ Primary | ✅ | ❌ | ✅ | ❌ | ✅ Primary |
| Zsh | ✅ Full | ✅ | ✅ Primary | ✅ | ✅ Primary | ❌ |
| Bash | ✅ Limited¹ | ✅ | ❌ | ✅ | ❌ | ❌ |
| PowerShell | ❌ | ✅ | ❌ | ✅ Primary | ❌ | ❌ |
| Nushell | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ |
| **Git Integration** | | | | | | |
| Branch name | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Ahead/behind | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Staged files | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Unstaged files | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Untracked files | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Stash count | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| Merge/rebase state | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| Worktree support | ✅ | ✅ | ✅ | ✅ | ❌ | ❓ |

¹ Bash 5.0+ recommended; some features require modern Bash. See [docs/user/bash-limitations.md](../docs/user/bash-limitations.md)

### Language Detection

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Detection Method** | Content-based | Content-based | Version managers | Content-based | ❌ None | Basic |
| **Languages Supported** | 14+ (expandable to 500+) | 40+ | ~15 (via asdf/rbenv/nvm) | 30+ | ❌ | ~10 |
| **Confidence Scoring** | ✅ **Yes (70% bytes, 30% files)** | ❌ No² | ❌ No | ❌ No² | ❌ | ❌ |
| **Configurable Filtering** | ✅ all/primary/top-N | ❌ No | ❌ No | ❌ No | ❌ | ❌ |
| **Confidence Threshold** | ✅ 0.0-1.0 (hide trace files) | ❌ No | ❌ No | ❌ No | ❌ | ❌ |
| **Version Detection** | ✅ Optional | ✅ | ✅ (via managers) | ✅ | ❌ | ✅ Limited |
| **Version Caching** | ✅ 24hr TTL | ✅ | ✅ | ❓ | ❌ | ❓ |
| **Polyglot Repo Handling** | ⭐⭐⭐⭐⭐ Excellent³ | ⭐⭐⭐ Good | ⭐⭐ Fair | ⭐⭐⭐ Good | ❌ | ⭐⭐ Fair |

² Starship and Oh-My-Posh show languages based on presence, not dominance
³ GPY's confidence scoring accurately identifies primary vs. secondary languages

### Performance (Benchmarked)

| Metric | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|--------|-----|----------|------|------------|------|------|
| **Git Status - Tiny (~200 files)** | **25.4ms** ✅ | 27.3ms | ❓⁴ | ❓ | ❓ | ❓ |
| **Git Status - Small (~1.4k files)** | **25.6ms** ✅ | 27.6ms | ❓⁴ | ❓ | ❓ | ❓ |
| **Git Status - Medium (~28k files)** | **33.1ms** ✅ | 35.0ms | ❓⁴ | ❓ | ❓ | ❓ |
| **Git Status - Large (~91k files)** | **37.5ms** ✅ | 39.3ms | ❓⁴ | ❓ | ❓ | ❓ |
| **Advantage vs Starship** | **+6.06% faster** | Baseline | ❓ | ❓ | ❓ | ❓ |
| **IPC Latency** | ~0.04ms | N/A⁵ | N/A | N/A | N/A | N/A |
| **Memory Usage (idle)** | <15MB | ~10-20MB⁶ | <5MB⁷ | ~15-25MB | <1MB⁸ | <5MB |
| **Cold Start Time** | <100ms | <200ms⁶ | ~50ms⁷ | ~150ms | Instant⁸ | Instant |

⁴ p10k claims to be very fast but no formal benchmarks available for comparison
⁵ Starship is synchronous (no separate agent process)
⁶ Estimated based on Rust binary typical footprint
⁷ Pure Zsh implementation (no external dependencies)
⁸ Pure shell implementation (minimal overhead)

**Benchmark Details:**
- Source: `gpy-agent/benchmarks/README.md`
- Date: 2025-12-02
- Method: 1000 runs per test on real-world repositories
- Statistical significance: p<0.001 for all measurements

### Architecture & Design

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Language** | Rust + Shell | Rust | Zsh + C | Go | Zsh | Fish |
| **Architecture** | Hybrid Agent | Synchronous | Async Zsh | Synchronous | Pure Shell | Pure Shell |
| **Background Agent** | ✅ Optional | ❌ No | ✅ Optional | ❌ No | ❌ No | ❌ No |
| **IPC Method** | Unix sockets | N/A | Zsh modules | N/A | N/A | N/A |
| **Git Backend** | Native git | libgit2 (Rust) | Native git | libgit2 (Go) | Native git | Native git |
| **Config Format** | TOML | TOML | Zsh vars | TOML/JSON | Zsh vars | Fish funcs |
| **Config Hot-Reload** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No | ❌ No |

### Live Updates & Reactivity

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **File Watching** | ✅ notify crate | ❌ No | ✅ gitstatus | ❌ No | ❌ No | ❌ No |
| **Async Updates** | ✅ SIGUSR1 | ❌ No | ✅ ZLE | ❌ No | ❌ No | ❌ No |
| **Config Changes** | ✅ Live | ✅ Live | ✅ Live | ✅ Live | Manual | Manual |
| **Debouncing** | ✅ 100ms | N/A | ✅ Yes | N/A | N/A | N/A |
| **Prompt Refresh Latency** | 50-250ms⁹ | On enter only | <100ms | On enter only | On enter only | On enter only |

⁹ Time from `git add` in terminal A to prompt update in terminal B

### Customization & Flexibility

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Themes** | ✅ TOML-based | ✅ TOML-based | ✅ Built-in wizard | ✅ JSON themes | ✅ Minimal | ✅ Built-in |
| **Custom Segments** | ✅ Shell functions | ✅ TOML config | ✅ Zsh functions | ✅ TOML config | ✅ Zsh functions | ✅ Fish functions |
| **Segment Order** | ✅ Configurable | ✅ Configurable | ✅ Configurable | ✅ Configurable | Fixed | ✅ Configurable |
| **Icons** | ✅ Nerd Fonts | ✅ Nerd Fonts | ✅ Nerd Fonts | ✅ Nerd Fonts | ❌ ASCII | ✅ Nerd Fonts |
| **Colors** | ✅ RGB/Named | ✅ RGB/Named | ✅ RGB/Named | ✅ RGB/Named | Basic | ✅ RGB/Named |
| **Delimiter Styles** | ✅ 4 built-in | ✅ Many | ✅ Powerline | ✅ Many | Minimal | ✅ Several |
| **CLI Tool** | ✅ `gpy` | ✅ `starship` | ✅ `p10k configure` | ✅ `oh-my-posh` | ❌ Manual | ✅ `tide configure` |

### Developer Experience

| Feature | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Installation** | Cargo/Script | Cargo/Package | git clone | Winget/Homebrew | git clone | Fisher/Package |
| **Documentation** | ✅ Extensive | ✅ Excellent | ✅ Good | ✅ Good | ✅ Minimal | ✅ Good |
| **Test Coverage** | 600+ tests | ~200 tests¹⁰ | Good | Good | Minimal | Good |
| **CI/CD** | ✅ GitHub Actions | ✅ GitHub Actions | ✅ | ✅ | ❌ | ✅ |
| **Contribution Guide** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes |
| **Plugin System** | 🔜 Planned | ✅ Custom modules | ✅ Plugins | ✅ Segments | ❌ No | ✅ Functions |

¹⁰ Estimated based on repository analysis

### Platform Support

| Platform | GPY | Starship | p10k | Oh-My-Posh | Pure | Tide |
|---------|-----|----------|------|------------|------|------|
| **Linux** | ✅ Full | ✅ Full | ✅ Full | ✅ Full | ✅ Full | ✅ Full |
| **macOS** | ✅ Full | ✅ Full | ✅ Full | ✅ Full | ✅ Full | ✅ Full |
| **Windows** | ✅ WSL only | ✅ Full | ❌ WSL only | ✅ Full | ❌ WSL only | ❌ WSL only |
| **ARM64** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes |
| **BSD** | ✅ Should work¹¹ | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Should work |

¹¹ Not formally tested but no platform-specific dependencies

---

## Feature Highlights by Tool

### 🐠 **GPY** (This Project)

**Unique Strengths:**
- ✅ **Confidence-based language detection** - Only prompt that scores languages by dominance
- ✅ **Smart filtering** - Show primary language only or top-N by confidence
- ✅ **Trace file suppression** - Hide languages below configurable threshold (e.g., <5%)
- ✅ **Live filesystem updates** - Prompt updates without pressing Enter (via file watching + SIGUSR1)
- ✅ **Proven faster than Starship** - 6% faster on benchmarked real-world repos (n=1000, p<0.001)
- ✅ **Hybrid architecture** - Best of both worlds (fast shell + powerful Rust agent)
- ✅ **Native git optimizations** - Leverages fsmonitor, untracked cache, sparse-checkout

**Best For:**
- Polyglot repositories (Rust + Node + Python projects)
- Users who want intelligent language detection
- Fish or Zsh power users
- Those who value performance + features

**Limitations:**
- Newer project (smaller community)
- Bash support has limitations (see docs)
- Windows requires WSL

---

### ⭐ **Starship**

**Unique Strengths:**
- ✅ Most cross-shell support (6+ shells including Nushell, PowerShell)
- ✅ Huge community and ecosystem
- ✅ Excellent documentation
- ✅ Mature, battle-tested codebase
- ✅ Works natively on Windows (no WSL required)

**Best For:**
- Users who switch between multiple shells
- Windows users who want native support
- Those who value stability and community

**Limitations:**
- Slower than GPY (6% on benchmarks)
- No confidence-based language detection (shows all present languages)
- No live updates (prompt only updates on Enter)
- Synchronous architecture (no background agent option)

---

### ⚡ **Powerlevel10k**

**Unique Strengths:**
- ✅ Extremely fast (highly optimized Zsh)
- ✅ Instant-prompt mode (cached rendering)
- ✅ Interactive configuration wizard
- ✅ Massive Zsh-specific optimizations
- ✅ Built-in gitstatus for fast git operations

**Best For:**
- Zsh users (only)
- Those who prioritize raw speed
- Users who want visual configuration wizard

**Limitations:**
- Zsh only (no Fish, Bash, etc.)
- Version manager-based language detection (not ideal for polyglot repos)
- No confidence scoring for languages
- Requires manual setup for each machine

---

### 🎨 **Oh-My-Posh**

**Unique Strengths:**
- ✅ Native Windows/PowerShell support
- ✅ Beautiful themes out of the box
- ✅ Good cross-platform consistency
- ✅ JSON theme format (easy to share)

**Best For:**
- Windows/PowerShell users
- Those who want pretty defaults
- Cross-platform developers (Windows + Linux)

**Limitations:**
- Heavier memory footprint
- No live updates
- No confidence-based language detection
- Slower than native-optimized prompts

---

### 💎 **Pure**

**Unique Strengths:**
- ✅ Extremely minimal and fast
- ✅ No external dependencies
- ✅ Pure Zsh (no compilation needed)
- ✅ Clean, uncluttered design

**Best For:**
- Minimalists
- Users who want zero dependencies
- Those who prioritize simplicity

**Limitations:**
- Zsh only
- Very basic language detection
- No live updates
- Limited customization options
- No advanced git features

---

### 🌊 **Tide**

**Unique Strengths:**
- ✅ Fish-native (written in Fish)
- ✅ Interactive configuration tool
- ✅ Good performance for pure Fish
- ✅ Clean, modern design

**Best For:**
- Fish-only users
- Those who want Fish-native solution
- Users who value simplicity

**Limitations:**
- Fish only (no Zsh, Bash)
- Basic language detection
- No confidence scoring
- No live updates
- Limited compared to multi-language solutions

---

## Methodology Notes

**Performance Benchmarks:**
- GPY vs Starship: Controlled local benchmarks on specific hosts using real-world repos (tiny to large)
- Other tools: Estimates based on architecture analysis and community reports
- All measurements should be validated in your environment
- Performance sections should be read as comparative diagnostics, not guarantees of exact latency on every user machine

**Feature Detection:**
- ✅ Confirmed through documentation or source code
- ❌ Confirmed absent
- ❓ Unknown (needs research)
- 🔜 Planned/In development

**Last Benchmark Update:** 2026-05-15 (IPC latency; real-world repo comparison last run 2025-12-02)
**Source:** `gpy-agent/benchmarks/README.md` and `docs/dev/performance/performance-baselines.md`

---

## Future Benchmark Plans

**Planned Comparisons:**
1. Formal benchmark suite against p10k (Zsh environment)
2. Memory usage comparison across all tools
3. CPU usage during prompt rendering
4. Cold start time analysis
5. Configuration reload performance
6. Large repository scalability tests (>100k files)

**Contributions Welcome:**
If you have benchmark data for other tools or can help run comparative tests, please open an issue or PR!

---

## Conclusion

**Choose GPY if you want:**
- Intelligent, confidence-based language detection
- Measured, actively monitored performance with regression tracking
- Live filesystem updates without pressing Enter
- Native git optimizations for large repos
- Fish or Zsh with modern features

**Choose Starship if you want:**
- Maximum cross-shell compatibility (including PowerShell, Nushell)
- Mature, stable codebase with huge community
- Native Windows support (no WSL)
- Extensive plugin ecosystem

**Choose Powerlevel10k if you want:**
- Maximum speed on Zsh
- Instant-prompt mode with caching
- Interactive configuration wizard
- Zsh-specific optimizations

**Choose Oh-My-Posh if you want:**
- Native Windows/PowerShell experience
- Beautiful themes out of the box
- Good cross-platform consistency

**Choose Pure/Tide if you want:**
- Minimal dependencies
- Pure shell implementation
- Simple, clean design
- Shell-native solution

---

**Questions or Corrections?**

This comparison is maintained as a living document. If you notice inaccuracies or have benchmark data to contribute, please [open an issue](https://github.com/jpease/gpy/issues) or submit a PR.
