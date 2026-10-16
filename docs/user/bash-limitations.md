# Bash Support: Limitations and Caveats

## Overview

GPY renders the same segments in Bash as in Fish and Zsh (git status, language detection, directory, status, clock), talks to the same agent, and applies config and theme changes. Bash differs in live updates, duration precision, and `PS1` syntax. The side-by-side comparison of all shells is the [Shell feature matrix](../INSTALL.md#shell-feature-matrix); this page explains the Bash rows.

---

## Limited Features

### 1. Command Duration Tracking

Precision depends on the Bash version:

| Bash Version | Duration Precision | Notes |
|--------------|-------------------|-------|
| **Bash 5.0+** | Millisecond | Uses `EPOCHREALTIME` (like Zsh) |
| **Bash 4.x** | Millisecond with GNU `date`; whole seconds with BSD/macOS `date` | Uses `date +%s%N`, which forks once per command. BSD/macOS `date` has no `%N` (it prints a literal `N`), so GPY probes `date` once at startup and falls back to whole-second timing there |
| **Bash 3.x** | Not supported | No sub-second timing; segment disabled |

**Workaround**: Upgrade to Bash 5 via Homebrew (macOS) or your package manager (Linux):

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
the idle prompt in place.

Bash installs no SIGURG trap. A trap could only re-render a `PS1`
readline cannot show: on Bash 5 a doorbell arriving during a render started a
nested one, so a steady stream of doorbells hung the shell, Bash 3.2 ran the
trap at an idle prompt and re-rendered for nothing, and every doorbell made a
running `wait` return early. SIGURG keeps its default disposition, ignore, so
it never interrupts a command or `wait` and never kills a shell. The agent's
`<pid>.reload` and `<pid>.reregister` flag files are read when the next prompt
is drawn instead.

**Mitigation**: Every prompt render reads the agent's current state, so the
change is never lost — it is one Enter away. The agent keeps its cache current
as files change, so that next prompt does not wait on git.

Bash re-registers after an agent restart, and applies a config reload, at its
next prompt: the agent leaves a `<pid>.reregister` or `<pid>.reload` flag, and
the prompt drawn after the next Enter consumes it before rendering. A dead
agent is recovered by the periodic supervisor check, so live updates resume
without reopening the shell.

---

### 3. Performance

Bash renders the prompt in shell code through `PROMPT_COMMAND`, and its string
and array handling is slower than Fish's or Zsh's. The render-time budgets
every gate enforces are in `tests/performance-baselines.json`
(`./scripts/bench.sh --ci` checks them; `./scripts/bench.sh` measures your
machine with hyperfine).

---

### 4. Prompt Customization

Bash's `PS1` uses different escape sequences:

| Feature | Zsh | Bash |
|---------|-----|------|
| Username | `%n` | `\u` |
| Hostname | `%m` | `\h` |
| Directory | `%~` | `\w` |
| Time | `%D{%H:%M}` | `\t` or `\A` |
| Colors | `%F{color}` | `\[\033[...m\]` |

GPY handles this internally, but custom `PS1` tweaks differ between shells.

---

## Unavailable Features

### Transient Prompt

Fish 4.2+ and Zsh collapse a submitted prompt to `<time> ❯ <command>` (`ui.transient_prompt`, on by default). Bash keeps the full prompt: readline cannot redraw a prompt it has already printed, and rewriting it with terminal control sequences risks breaking terminal state.

### Clock Time In The Scrollback

Bash expands the clock's `\D{...}` when it draws the prompt, so a prompt left waiting before Enter keeps the time it appeared. Fish 4.2+ and Zsh redraw the prompt when Enter is pressed and show that time instead.

---

## Version Recommendations

- **Bash 3.2+** required (macOS default)
- **Bash 5.0+** recommended: it is the only Bash with millisecond durations without forking `date`

GPY detects the Bash version at startup and disables what that version cannot support:

```bash
bash --version
# GNU bash, version 5.3.8(1)-release   <- full features
# GNU bash, version 3.2.57(1)-release  <- macOS default, no duration segment
```

### Migration Path

If you're on Bash 3.2 (macOS):

```bash
# Option 1: Upgrade Bash
brew install bash
chsh -s /opt/homebrew/bin/bash

# Option 2: Switch to Zsh (ships with macOS)
chsh -s /bin/zsh

# Option 3: Switch to Fish
brew install fish
chsh -s /opt/homebrew/bin/fish
```

---

## Known Issues

### Duration shows "0ms" on the first prompt
**Bash version**: All
**Cause**: `PROMPT_COMMAND` hasn't timed a command yet
**Workaround**: Displays correctly after the first command

### Agent updates during a command show at the next prompt
**Bash version**: All
**Cause**: Bash ignores the agent's SIGURG doorbell; its flags are read when the next prompt is drawn
**Workaround**: None needed: the prompt after the command shows the current state

### Prompt renders slowly on Bash 3.2
**Bash version**: 3.x
**Cause**: Slow string operations
**Workaround**: Upgrade to Bash 5 or switch shells
