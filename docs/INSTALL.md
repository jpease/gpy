# GPY Installation Guide

This guide covers all methods for installing GPY on your system.

## Table of Contents

- [Supported Platforms and Shells](#supported-platforms-and-shells)
- [One-Line Installation (Recommended)](#one-line-installation-recommended)
- [Verifying Your Download](#verifying-your-download)
- [Manual Installation](#manual-installation)
- [Building from Source](#building-from-source)
- [Troubleshooting](#troubleshooting)
- [Uninstallation](#uninstallation)

## Supported Platforms and Shells

This is the canonical statement of what GPY supports. Every other document
(README, release notes, CLI help, Cargo metadata) points here rather than
restating it.

### Operating systems

| OS | Support |
|----|---------|
| Linux (x86_64, aarch64) | Release binaries need glibc 2.31 or newer (e.g. Ubuntu 20.04+, Debian 11+); the release job rejects a binary that references a newer glibc and runs the x86_64 binaries in `ubuntu:20.04` (`GPY_GLIBC_FLOOR` in `.github/workflows/release.yml`, #694). Build- and unit-tested in CI; a fresh install into an empty `$HOME` on a stock `ubuntu:24.04` container renders a first prompt in fish, zsh and bash (`scripts/test-fresh-install.sh`). The shell end-to-end tier has not yet had a recorded green Linux run in Actions (#651 tracks the first one, after which this row becomes "Full") |
| macOS 11+ (Intel and Apple Silicon) | Full: every tier, including the shell end-to-end suites, runs on the maintainer's pre-push gate |
| Windows, via WSL | Recommended — run the Linux instructions inside WSL |
| Windows, native | CLI-only; the prompt integration does not run there. The claim is backed by the five CLI integration targets the Windows gate runs (`gpy_cli_tests`, `cli_config_mutation_tests`, `theme_import_tests`, `init_command_tests`, `cli_integration_tests`; `.github/workflows/windows-gate.yml`), once that leg has a recorded green run (#653). See [Windows (WSL)](#windows-wsl) below for what "not supported" covers. |

What CI actually backs: `pr-gate.yml` and `cross-platform-test.yml` share one
`windows-latest` leg (`windows-gate.yml`) that builds the agent with `cargo
build --locked`, runs the library unit tests with `cargo test --locked --lib`,
and, since #653, the five CLI integration targets listed in the table above
through nextest. As of 2026-09-02 that leg is no longer
gated on `pull_request` — the workflow's automatic trigger is temporarily
disabled to control metered GitHub Actions minutes on this private repo
(issue #556). `cross-platform-test.yml`'s `push`-to-`main` and weekly-cron
legs are disabled for the same reason, so Windows coverage now comes only
from dispatching either workflow manually (`gh workflow run pr-gate.yml`)
before merging. So the Windows **build and library unit tests** are covered,
but not as a required gate on the PR itself while this holds.

What CI does not cover on native Windows is the shell integration, and it
never will: the prompt talks to the agent over a Unix domain socket, which
native Windows does not provide. `pr-gate.yml`'s Windows leg deliberately runs
no Fish, Bash or Zsh suites for that reason. End-to-end behaviour on native
Windows is unmeasured (#513), and one library test —
`config::manager::tests::poll_fallback_reloads_without_any_watcher_running` —
is a known intermittent timing flake on that leg; re-run once before treating
a red result as a regression.

### Shells

| Shell | Minimum version |
|-------|------------------|
| Fish | 3.6+ |
| Zsh | 5.8+ |
| Bash | 4.0+ (5.0+ recommended — see [Bash on macOS is too old](#bash-on-macos-is-too-old)) |

### IPC transport dependencies

Each shell talks to the `gpy-agent` daemon over a Unix domain socket. What
that requires varies by shell:

| Shell | Required / recommended tools |
|-------|-------------------------------|
| Zsh | None required — the `zsh/net/socket` builtin module (`zsocket`) talks to the socket directly. Falls back to `socat`, then `nc -U` (which uses `awk` to compute a fractional `timeout` bound when `timeout` is present). |
| Bash | `socat`, or `nc -U` (wrapped in `timeout` when available). No built-in fallback. |
| Fish | `socat`, or `nc -U` (wrapped in `timeout` when available). No built-in fallback. |

### Degraded (oneshot) behavior

When none of the above is available, or the `gpy-agent` daemon isn't
running, all three shells fall back to forking `gpy-agent oneshot` for that
prompt render. A PID-scoped marker file limits this to one fork per prompt
render, so a broken IPC path costs one extra process per prompt rather than
one per segment. The background/live-refresh path has no such fallback:
without a running daemon there is no `SIGURG` repaint, so the prompt stays
static until the next render.

## One-Line Installation (Recommended)

The easiest way to install GPY is with the one-line installer:

```bash
curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | sh
```

### What it does

The installer automatically:

1. **Detects your shell** (Fish, Zsh, or Bash)
2. **Downloads the binary** for your platform (Linux/macOS, x86_64/aarch64/arm64)
   from a resolved release tag, and **verifies its SHA-256 checksum** against
   the digest published with the release. A missing or mismatched checksum
   aborts the install without touching your existing setup.
3. **Installs files** to appropriate locations:
   - Agent binary: `~/.local/bin/gpy-agent`
   - Fish: `~/.config/fish/gpy/`
   - Zsh: `~/.config/gpy/zsh/`
   - Bash: `~/.config/gpy/bash/`
4. **Configures your shell** by adding source lines to:
   - Fish: `~/.config/fish/config.fish`
   - Zsh: `${ZDOTDIR:-~}/.zshrc`
   - Bash: `~/.bashrc` (created if missing). If your login file (the first of `~/.bash_profile`, `~/.bash_login`, `~/.profile`) does not already load `~/.bashrc`, a small loader block is added to it; if none exists, `~/.profile` is created
5. **Starts the agent** (if not already running)

### Requirements

- **Operating System / Shell**: see [Supported Platforms and Shells](#supported-platforms-and-shells)
- **Network Tools**: `curl` or `wget`
- **Checksum Tool**: `sha256sum` or `shasum` (both ship with Linux and macOS;
  the installer refuses to run without one, because it will not install a
  binary it cannot verify)
- **Optional**: Nerd Fonts for icons

### Environment Variables

Customize the installation behavior:

- **`GPY_SHELL`**: Override shell detection
  ```bash
  curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | GPY_SHELL=fish sh
  ```

- **`GPY_VERSION`**: Install a specific version
  ```bash
  curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | GPY_VERSION=v0.1.0 sh
  ```

- **`GPY_NERD_FONT`**: Force the icon style instead of auto-detecting (see
  [Icons and Nerd Fonts](#icons-and-nerd-fonts)). Accepts `1`/`nerd`, `0`/`none`,
  or `unknown`.
  ```bash
  curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | GPY_NERD_FONT=none sh
  ```

### After Installation

Restart your shell to activate GPY:

```bash
# Fish
exec fish

# Zsh
exec zsh

# Bash
exec bash
```

Or reload your configuration:

```bash
# Fish
source ~/.config/fish/config.fish

# Zsh
source "${ZDOTDIR:-$HOME}/.zshrc"

# Bash
source ~/.bashrc
```

### Icons and Nerd Fonts

GPY's default prompt uses [Nerd Font](https://www.nerdfonts.com) glyphs (git
state, language icons, powerline separators). On a terminal that is **not**
using a Nerd Font, those glyphs render as tofu (`□`) or `?` — so the installer
runs `gpy-agent init` to pick an icon style that renders on your machine before
your first prompt:

- **A Nerd Font is detected** → icons stay on (`ui.show_icons = true`). When a
  terminal is attached, GPY also shows a sample line and asks you to confirm the
  glyphs actually render (font presence alone doesn't prove your terminal is
  configured to use it).
- **No Nerd Font is detected, or detection is inconclusive** → GPY defaults to
  **ASCII** icons (`ui.show_icons = false`), which render on any terminal, and
  prints how to enable the full set later.

`gpy-agent init` never overwrites an existing config, so upgrades and
already-configured users keep their settings. Change the style at any time:

```bash
gpy config set ui.show_icons true    # full Nerd Font glyphs
gpy config set ui.show_icons false   # ASCII fallback
```

To install a Nerd Font, download one from [nerdfonts.com](https://www.nerdfonts.com),
install it, and set it as your terminal's font. Then re-run
`gpy config set ui.show_icons true` (or reinstall) to switch on the glyphs.

You can also force the choice with the `GPY_NERD_FONT` environment variable
(`1`/`nerd`, `0`/`none`, or `unknown`) — useful for provisioning scripts and
non-interactive installs. A set override is taken as your answer: `gpy-agent
init` writes `show_icons` from it and never opens the confirmation prompt,
even when a terminal is attached.

### Shell Completions

Tab-completion for the `gpy` CLI is installed automatically as part of the standard installation flows — no manual step needed. Completions are regenerated on every install or upgrade, ensuring they match your installed binary.

Completion types:
- `gpy <TAB>` — Subcommands and flags
- `gpy theme use <TAB>`, `gpy theme validate <TAB>` — Installed theme names
- `gpy palette use <TAB>`, `gpy palette validate <TAB>` — Installed palette names
- `gpy enable <TAB>`, `gpy disable <TAB>` — Available segment names

## Verifying Your Download

Every release publishes a `.sha256` sidecar next to each downloadable file, an
aggregate `SHA256SUMS` manifest, and a CycloneDX SBOM (`gpy-sbom.cdx.json`).
The installers verify checksums for you; these steps are for verifying a
manual download, or for confirming what an installer did.

**Checksum:**

```bash
VERSION=v0.1.0   # the release you downloaded
BASE=https://github.com/jpease/gpy/releases/download/$VERSION

curl -fsSLO "$BASE/SHA256SUMS"
sha256sum --ignore-missing -c SHA256SUMS     # macOS: shasum -a 256 --ignore-missing -c
```

A single file can be checked against its own sidecar instead:

```bash
curl -fsSLO "$BASE/gpy-agent-linux-x86_64.sha256"
sha256sum -c gpy-agent-linux-x86_64.sha256
```

**Provenance:** each artifact carries a GitHub build attestation tying it to
the workflow run and commit that produced it. With the [GitHub
CLI](https://cli.github.com/):

```bash
gh attestation verify gpy-agent-linux-x86_64 --repo jpease/gpy
```

GPY binaries are not code-signed or notarized. See
[SECURITY.md](../SECURITY.md#code-signing-not-done-deliberately) for what that
means in practice on macOS and Windows.

## Manual Installation

For more control over the installation process:

### 1. Download and Verify a Pre-Built Binary

Download the appropriate binary for your platform from the [releases page](https://github.com/jpease/gpy/releases),
along with its checksum sidecar. Substitute the asset name for your platform:
`gpy-agent-linux-x86_64`, `gpy-agent-linux-aarch64`, `gpy-agent-macos-aarch64`
(Apple Silicon) or `gpy-agent-macos-x86_64` (Intel).

```bash
VERSION=v0.1.0
ASSET=gpy-agent-linux-x86_64
BASE=https://github.com/jpease/gpy/releases/download/$VERSION

wget "$BASE/$ASSET" "$BASE/$ASSET.sha256"
sha256sum -c "$ASSET.sha256"     # macOS: shasum -a 256 -c "$ASSET.sha256"

chmod +x "$ASSET"
mv "$ASSET" ~/.local/bin/gpy-agent
```

Do not skip the checksum step. It is the only thing standing between a
corrupted or substituted download and a binary that runs on every prompt.

The `gpy` CLI, which powers shell completions and subcommands, ships as a
parallel asset (`gpy-linux-x86_64`, `gpy-macos-aarch64`, and so on) and
installs the same way to `~/.local/bin/gpy`.

### 2. Install Shell Files

Clone the repository to get shell integration files:

```bash
git clone https://github.com/jpease/gpy.git /tmp/gpy
```

**For Fish:**
```bash
mkdir -p ~/.config/fish/gpy
cp -r /tmp/gpy/fish/* ~/.config/fish/gpy/

# Add to config.fish. Source conf.d/gpy_init.fish, the same fail-safe
# entry point the installers use (it checks the layout before loading and
# disables GPY cleanly when something is missing); the marker lines are
# what scripts/uninstall.fish removes.
echo '
# >>> gpy-init >>>
# GPY Prompt Enhancement
if status is-interactive
    source ~/.config/fish/gpy/conf.d/gpy_init.fish
end
# <<< gpy-init <<<' >> ~/.config/fish/config.fish

# Link prompt function
mkdir -p ~/.config/fish/functions
ln -sf ~/.config/fish/gpy/functions/fish_prompt.fish ~/.config/fish/functions/fish_prompt.fish
```

**For Zsh:**
```bash
mkdir -p ~/.config/gpy/zsh
cp -r /tmp/gpy/zsh/* ~/.config/gpy/zsh/

# Add to .zshrc (the marker lines are what scripts/uninstall.sh removes)
printf '\n# >>> gpy-init >>>\n# GPY Prompt Enhancement\nsource ~/.config/gpy/zsh/gpy.zsh\n# <<< gpy-init <<<\n' >> "${ZDOTDIR:-$HOME}/.zshrc"
```

**For Bash:**
```bash
mkdir -p ~/.config/gpy/bash
cp -r /tmp/gpy/bash/* ~/.config/gpy/bash/

# Add to .bashrc (the marker lines are what scripts/uninstall.sh removes)
printf '\n# >>> gpy-init >>>\n# GPY Prompt Enhancement\nsource ~/.config/gpy/bash/gpy.bash\n# <<< gpy-init <<<\n' >> ~/.bashrc
```

### 3. Verify Installation

```bash
# Check agent is installed
gpy-agent --version

# Check agent status
gpy-agent status

# Start agent if not running
gpy-agent start

# Restart shell
exec fish  # or zsh, bash
```

## Building from Source

If you want to build GPY yourself or contribute to development:

### Prerequisites

- **Rust**: 1.98 or later — the `rust-version` declared in `gpy-agent/Cargo.toml`, which is what `cargo build` enforces
- **Git**: 2.0 or later
- **Shell**: see [Supported Platforms and Shells](#supported-platforms-and-shells)

### Build Steps

```bash
# Clone repository
git clone https://github.com/jpease/gpy.git
cd gpy

# Build the agent
cd gpy-agent
cargo build --release
cd ..

# The binary is now at: gpy-agent/target/release/gpy-agent
```

### Install Locally

`install.sh` is the release-archive installer: it verifies and installs the
checksummed `bin/` payload a release tarball carries, which a checkout does
not contain, so it cannot be used from source. Use the per-shell steps below
instead.

**Fish** — `install-dev.fish` builds the agent (skip the build above; or pass
`--no-build` to reuse it), installs both binaries into `~/.local/bin`, and
wires up the Fish integration, completions and prompt:

```bash
fish install-dev.fish
```

**Zsh and Bash** — install the binaries you built, then copy the shell
integration and source it from your rc file:

```bash
# Install both binaries (the agent daemon and the gpy CLI)
mkdir -p ~/.local/bin
install -m 755 gpy-agent/target/release/gpy-agent gpy-agent/target/release/gpy ~/.local/bin/

# For Zsh:
mkdir -p ~/.config/gpy/zsh
cp -r zsh/* ~/.config/gpy/zsh/
printf '\n# >>> gpy-init >>>\n# GPY Prompt Enhancement\nsource ~/.config/gpy/zsh/gpy.zsh\n# <<< gpy-init <<<\n' >> "${ZDOTDIR:-$HOME}/.zshrc"

# For Bash:
mkdir -p ~/.config/gpy/bash
cp -r bash/* ~/.config/gpy/bash/
printf '\n# >>> gpy-init >>>\n# GPY Prompt Enhancement\nsource ~/.config/gpy/bash/gpy.bash\n# <<< gpy-init <<<\n' >> ~/.bashrc
```

Make sure `~/.local/bin` is on your `PATH` (see
[Troubleshooting](#agent-binary-not-found)). Every command in this section is
executed by `tests/bash/install_from_source_docs.test.bash` from a clean
checkout, so it cannot drift from what works.

## Troubleshooting

### Agent binary not found

If you get "command not found" errors:

1. Check if `~/.local/bin` is in your `PATH`:
   ```bash
   echo $PATH
   ```

2. If not, add it to your shell profile:
   ```bash
   # For Fish (in config.fish)
   fish_add_path ~/.local/bin

   # For Zsh (in .zshrc)
   export PATH="$HOME/.local/bin:$PATH"

   # For Bash (in .bashrc)
   export PATH="$HOME/.local/bin:$PATH"
   ```

### Agent won't start

Check the logs:

```bash
cat ~/.cache/gpy/agent.log
```

Common issues:
- **Socket permission errors**: Delete the socket `gpy status` reports (`Socket Path: ...`, normally `$XDG_RUNTIME_DIR/gpy/gpy.sock`, else `~/.cache/gpy/gpy.sock`) and retry
- **Port already in use**: Another agent instance is running, check with `gpy-agent status`

### Prompt not updating

1. Check if agent is running:
   ```bash
   gpy-agent status
   ```

2. Restart the agent:
   ```bash
   gpy-agent stop
   gpy-agent start
   ```

3. Verify shell files are sourced:
   ```bash
   # Fish
   grep -i gpy ~/.config/fish/config.fish

   # Zsh
   grep -i gpy "${ZDOTDIR:-$HOME}/.zshrc"

   # Bash
   grep -i gpy ~/.bashrc
   ```

### macOS "Permission Denied" or "Operation not permitted"

macOS Gatekeeper may block the downloaded binary, because GPY binaries are not
notarized (see [SECURITY.md](../SECURITY.md#code-signing-not-done-deliberately)).
The installers clear the quarantine flag for you after verifying the binary's
checksum. For a manual install, verify the checksum first, then:

```bash
# Remove the quarantine attribute (only after `sha256sum -c` has passed)
xattr -d com.apple.quarantine ~/.local/bin/gpy-agent

# Or allow it in System Settings:
# System Settings → Privacy & Security → "Allow Anyway"
```

### Bash on macOS is too old

macOS ships with Bash 3.2 (from 2007). GPY requires Bash 4.0+:

```bash
# Install modern Bash via Homebrew
brew install bash

# Verify version
bash --version
```

## Uninstallation

Run either uninstall script (`fish scripts/uninstall.fish` or `sh scripts/uninstall.sh`).
Uninstallation is **global for the current user's install**: either script stops
the agent and supervisor processes, removes the binaries and their upgrade backups
from `~/.local/bin`, removes all shell integration files and completions (Fish,
Zsh, Bash), cleans the `gpy-init` block from all supported startup files
(`~/.bashrc`, `~/.bash_profile`, `~/.bash_login`, `~/.profile`, `${ZDOTDIR:-~}/.zshrc`, `~/.zshrc`, and `~/.config/fish/config.fish`), and
removes the config, cache, and runtime directories. It asks for confirmation first,
and startup files are restored byte-for-byte (startup files the installer created are removed).

```bash
# From a release archive or a clone of the repository
fish scripts/uninstall.fish                 # Fish entry point (uninstalls globally)
sh scripts/uninstall.sh                     # POSIX shell entry point (uninstalls globally)

# Installed with the one-liner and no local copy? Download the script first
# (piping it straight into a shell would feed the confirmation prompt the
# script itself):
curl -fsSL https://raw.githubusercontent.com/jpease/gpy/main/scripts/uninstall.sh -o /tmp/gpy-uninstall.sh
sh /tmp/gpy-uninstall.sh                    # or download uninstall.fish
```

### Fish Prompt Restoration

If your `fish_prompt.fish` is an unrelated custom prompt or symlink, the uninstaller
preserves it byte-for-byte and leaves all backups untouched.

When the prompt destination is absent (or becomes absent after removing the GPY
symlink/implementation), the uninstaller restores the newest backup by comparing the
timestamp suffix (`YYYYMMDD_HHMMSS`) across all accepted naming conventions:
- `fish_prompt.fish.backup.<stamp>` (from `install-oneline.sh` and `install-dev.fish`)
- `fish_prompt.fish.gpy-backup.<stamp>` (from `install.sh`)
- `fish_prompt.fish.backup-YYYYMMDD-HHMMSS` (legacy, from earlier `install-dev.fish` runs)

If timestamps are equal, `.gpy-backup.` is preferred deterministically. Older backups
remain untouched.

Every installer moves an existing prompt that is not GPY's own link aside with `mv`,
so a symlinked prompt (stow, yadm, chezmoi) stays a symlink to the same dotfile and is
restored as one. A backup that is a symlink into GPY's own prompt directory is never
restored.

### Manual Removal Checklist

To remove GPY manually instead, this is everything the uninstaller touches:

```bash
gpy-agent stop
rm -f ~/.local/bin/gpy-agent ~/.local/bin/gpy ~/.local/bin/gpy-agent.backup.* ~/.local/bin/gpy.backup.*
rm -rf ~/.config/fish/gpy ~/.config/fish/conf.d/gpy_init.fish ~/.config/fish/completions/gpy.fish ~/.config/fish/completions/gpy-dynamic.fish
rm -rf ~/.config/gpy                                           # config, plus Zsh/Bash integrations and completions
rm -rf ~/.cache/gpy "${XDG_RUNTIME_DIR:-~/.cache}/gpy"          # cache; socket, agent.version, shells/
# Remove the "# >>> gpy-init >>>" ... "# <<< gpy-init <<<" block from
# ~/.config/fish/config.fish, ${ZDOTDIR:-~}/.zshrc, ~/.zshrc, ~/.bashrc, ~/.bash_profile, ~/.bash_login, and ~/.profile
# Fish prompt: if a GPY symlink, remove it and restore the newest backup if desired
rm -f ~/.config/fish/functions/fish_prompt.fish
# Restore newest backup if present:
# mv ~/.config/fish/functions/fish_prompt.fish.backup.<newest_stamp> ~/.config/fish/functions/fish_prompt.fish

exec fish  # or zsh, bash
```

## Platform-Specific Notes

### Linux

- Release binaries need glibc 2.31 or newer: Ubuntu 20.04+, Debian 11+, Fedora 32+, RHEL/Rocky 9+, and current Arch Linux. The release job enforces the floor and runs the x86_64 binaries on Ubuntu 20.04; the other distros are not run in CI. Older or musl-based systems (e.g. Alpine) need a build from source
- Works on both x86_64 and aarch64 (ARM64)
- SELinux may require additional permissions for the socket

### macOS

- Supported on macOS 11 (Big Sur) and later
- Both Intel (x86_64) and Apple Silicon (arm64) are supported
- Gatekeeper may require manual approval on first run
- Bash 5.0+ recommended (via Homebrew), default 3.2 has limitations

### Windows (WSL)

- GPY runs in WSL (Windows Subsystem for Linux)
- Use the Linux installation instructions within WSL
- Native Windows (PowerShell) is not supported

## Support

For issues, questions, or feature requests:

- **GitHub Issues**: https://github.com/jpease/gpy/issues
- **Documentation**: https://github.com/jpease/gpy/tree/main/docs
- **Discussions**: https://github.com/jpease/gpy/discussions
