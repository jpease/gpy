# GPY Installation Guide

This guide covers all methods for installing GPY on your system.

## Table of Contents

- [Supported Platforms and Shells](#supported-platforms-and-shells)
- [One-Line Installation (Recommended)](#one-line-installation-recommended)
- [Homebrew](#homebrew)
- [`gpy setup`](#gpy-setup)
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
| Linux (x86_64, aarch64) | Full on x86_64: every tier, including the shell end-to-end suites (Fish, Bash and Zsh against a real agent on a pseudo-terminal), runs on every pull request on `ubuntu-latest` (Fish 4, zsh, bash), and a fresh install into an empty `$HOME` on a stock `ubuntu:24.04` container renders a first prompt in fish, zsh and bash (`scripts/test-fresh-install.sh`). aarch64 binaries are built and checked against the glibc floor but not run in CI. Release binaries need glibc 2.31 or newer (e.g. Ubuntu 20.04+, Debian 11+); the release job rejects a binary that references a newer glibc and runs the x86_64 binaries in `ubuntu:20.04` (`GPY_GLIBC_FLOOR` in `.github/workflows/release.yml`) |
| macOS 11+ (Intel and Apple Silicon) | Full: every tier, including the shell end-to-end suites, runs on every pull request on `macos-latest` (Apple Silicon). Intel binaries are built and checksummed in the release but only the Apple Silicon ones are run |
| Windows, via WSL | Recommended — run the Linux instructions inside WSL. `.github/workflows/wsl-smoke.yml` builds the agent in Ubuntu 24.04 under WSL 2 on a Windows runner and runs `scripts/wsl-smoke.sh` and the cross-shell contract suite there, when its own files change, weekly and on demand (it is not part of the PR gate). See [Windows (WSL)](#windows-wsl) for what is WSL-specific |
| Windows, native | CLI-only; the prompt integration does not run there. The claim is backed by the six CLI integration targets the Windows gate runs on every pull request (`gpy_cli_tests`, `cli_config_mutation_tests`, `config_concurrent_save_tests`, `theme_import_tests`, `init_command_tests`, `cli_integration_tests`; `.github/workflows/windows-gate.yml`). See [Windows (WSL)](#windows-wsl) below for what "not supported" covers. |

What CI actually backs: `pr-gate.yml` runs on every pull request to `main`
(and on manual dispatch), and `cross-platform-test.yml` runs after every push
to `main`, weekly, and on manual dispatch. The PR gate runs the Rust gate
(format, strict clippy, nextest, doc tests, `cargo audit`, `cargo deny`, a
release build), the strict `just lint` layer, and the Fish, Bash and Zsh
integration suites on `ubuntu-latest` and `macos-latest`, each as its own job
so one failure never hides another's verdict; a clean-container fresh install
on `ubuntu:24.04`; and the Windows leg. The one check to require in the branch
ruleset is `gate-result`, which fails unless every one of those jobs
succeeded. The PR gate runs on pull requests from forks with a read-only
token and no secrets.

The Windows leg is one reusable workflow (`windows-gate.yml`) that
`pr-gate.yml` and `cross-platform-test.yml` both call. It builds the agent
with `cargo build --locked`, runs the library unit tests with `cargo test
--locked --lib`, and the six CLI integration targets listed in the table
above through nextest.

What CI does not cover on native Windows is the shell integration, and it
never will: the prompt talks to the agent over a Unix domain socket, which
native Windows does not provide. `pr-gate.yml`'s Windows leg deliberately runs
no Fish, Bash or Zsh suites for that reason. End-to-end behaviour on native
Windows is unmeasured.

### Shells

| Shell | Minimum version | What backs the floor |
|-------|-----------------|----------------------|
| Fish | 3.6+ | Enforced at startup (`fish/core/init.fish`); CI runs Fish 4 |
| Zsh | 5.8+ | The 5.8 floor is untested; CI runs Zsh 5.9 |
| Bash | 3.2+ (5.0+ recommended — see [Bash on macOS is too old](#bash-on-macos-is-too-old)) | The debug-trap and `PROMPT_COMMAND` status-preservation tests also run under macOS `/bin/bash` 3.2; the other suites run Bash 5 |

### Shell feature matrix

Every shell renders the same built-in segments (directory, git, language,
status, clock) and applies config and theme hot-reloads. The differences:

| Feature | Fish | Zsh | Bash 5.x | Bash 4.x | Bash 3.2 (macOS default) |
|---|---|---|---|---|---|
| Live updates | Idle prompt repaints in place | Idle prompt repaints in place (`TRAPURG` + `zle reset-prompt`) | Shown at the next prompt (readline cannot repaint an idle prompt) | Same as Bash 5.x | Same as Bash 5.x |
| Signal mechanism | One `--on-signal SIGURG` handler | Native `TRAPURG` function | No trap: SIGURG stays ignored, flag files are read at the next prompt | Same as Bash 5.x | Same as Bash 5.x |
| Re-registers after an agent restart | Immediately | Immediately | At the next prompt | At the next prompt | At the next prompt |
| Plugin segments (v1 `.fish` files) | Loaded | Skipped (see [Plugin Framework](dev/plugins.md#segment-runtime-contract)) | Skipped | Skipped | Skipped |
| Duration segment | Millisecond | Millisecond (`EPOCHREALTIME`) | Millisecond (`EPOCHREALTIME`) | `date +%s%N` with GNU `date`; whole seconds with BSD `date` | Disabled |
| Transient prompt (`ui.transient_prompt`) | Fish 4.2+: a submitted prompt collapses to `<time> ❯ <command>` | Same as Fish (`zle-line-finish`) | No: the full prompt stays | Same as Bash 5.x | Same as Bash 5.x |
| Clock time in the scrollback | When Enter was pressed (Fish 4.2+; older Fish: when drawn) | When Enter was pressed | When the prompt was drawn | Same as Bash 5.x | Same as Bash 5.x |

Bash details (version detection, `PS1` differences, known issues) are in
[Bash Limitations](user/bash-limitations.md).

### IPC transport dependencies

Each shell talks to the `gpy-agent` daemon over a Unix domain socket. What
that requires varies by shell:

| Shell | Required / recommended tools |
|-------|-------------------------------|
| Zsh | None required — the `zsh/net/socket` builtin module (`zsocket`) talks to the socket directly. Falls back to `socat`, then `nc -U`. |
| Bash | `socat`, or `nc -U`. No built-in fallback. |
| Fish | `socat`, or `nc -U`. No built-in fallback. |

The same capability probe runs in all three shells: `socat` is used when it
exists; otherwise `nc` is used only if `nc -h` lists `-U`. A netcat without
`-U` (netcat-traditional, the `nc` on some minimal Debian and Ubuntu images)
counts as no client at all, so install `socat` or `netcat-openbsd` there.

Other tools GPY uses opportunistically, and what happens without them:

| Tool | Used for | Without it |
|------|----------|------------|
| `timeout` (GNU coreutils) | Wraps the `nc` client in Bash, Zsh and Fish so a silent agent is abandoned after `GPY_IPC_TIMEOUT_MS` (150 ms by default). Never needed with `socat` or `zsocket`, which take the budget directly. | Not required, and stock macOS does not ship it. `nc`'s own `-w` takes whole seconds, so a connected-but-silent agent can hold the prompt for up to one second instead of 150 ms. |
| `lsof` (or `fuser`) | Fish only. When `gpy-agent stop` leaves a lingering agent on the socket, an agent restart finds its process with `lsof -t` (`fuser` is the fallback) and kills it. | Neither is installed on many minimal Debian, Ubuntu and WSL images. The lingering agent is then left running and its socket file is kept; `gpy-agent start` reclaims a stale socket and evicts a wedged agent itself. With `GPY_DEBUG=1` the restart logs a warning naming the missing tools. |
| `awk` | Only a fallback: Bash 5 computes a command's duration from `EPOCHREALTIME` in shell arithmetic and calls `awk` just when that value is malformed. The IPC paths use no `awk`; millisecond budgets are converted in shell arithmetic. | Duration falls back to unavailable for that one command. |
| GNU `date` | Bash 4.x only: durations use `date +%s%N`. | BSD/macOS `date` has no `%N`; GPY probes for it once at startup and falls back to whole-second timing (a ~2 s command reads 1–3 s). Bash 5 uses `EPOCHREALTIME` and needs no `date`. |

Unix socket paths are limited to 103 bytes on macOS (`sun_path` is 104 bytes
including the NUL) and 107 on Linux. A long `$HOME`, `XDG_RUNTIME_DIR` or
`GPY_AGENT_SOCKET_PATH` therefore fails on macOS first; `gpy-agent start`,
`stop` and `status` refuse such a path with an error naming its length and
the limit. Point `XDG_RUNTIME_DIR` at a shorter directory or set
`GPY_AGENT_SOCKET_PATH` to a short path in a directory only you can write,
such as `/tmp/gpy-$USER/gpy.sock` after `mkdir -m 700 /tmp/gpy-$USER`. A
socket directly in `/tmp` is refused.

### Degraded (oneshot) behavior

When none of the above is available, or the `gpy-agent` daemon isn't
running (nothing listens on the socket), all three shells fall back to
forking `gpy-agent oneshot` for that prompt render. A per-render budget
(a plain shell variable, never a file) limits this to one fork per prompt
render, so a broken IPC path costs one extra process per prompt rather than
one per segment. A running agent that merely answers later than
`GPY_IPC_TIMEOUT_MS` is not treated as down: the shell has already sent it the
request, so that segment is omitted for the render instead of being recomputed
by a second, blocking process. The background/live-refresh path has no
oneshot fallback: without a running daemon there is no `SIGURG` repaint, so
the prompt stays static until the next render.

## One-Line Installation (Recommended)

The one-line installer is an asset of the latest release (not a file on a
branch):

```bash
curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh
```

To install a specific release, use that release's own copy of the installer. It
has the release tag built in and installs exactly that release (replace
`vX.Y.Z` with a release tag):

```bash
curl -fsSL https://github.com/jpease/gpy/releases/download/vX.Y.Z/install-oneline.sh | sh
```

### What it does

The installer:

1. **Detects your shell** (Fish, Zsh, or Bash)
2. **Downloads the binary** for your platform (Linux/macOS, x86_64/aarch64/arm64)
   from the release the installer was published with, and **verifies its
   SHA-256 checksum** against the digest published with that release. A
   missing or mismatched checksum aborts the install without touching your
   existing setup.
3. **Installs the `gpy` CLI** (also checksum-verified). It is required: the
   installer hands the shell files it unpacked to [`gpy setup`](#gpy-setup),
   so the curl, Homebrew and source installs all finish the same way.

   The shell files come from one archive, `gpy-shell-files.tar.gz`, that the
   installer downloads and checks against its published digest before it
   unpacks anything, so every file it installs is covered by a verified digest.
4. **Runs `gpy setup`**, which installs the shell files (the binaries go to
   `~/.local/bin/gpy-agent` and `~/.local/bin/gpy`):
   - Fish: `~/.config/fish/gpy/`
   - Zsh: `~/.config/gpy/zsh/`
   - Bash: `~/.config/gpy/bash/`

   and configures your shell by adding a marked block to:
   - Fish: `~/.config/fish/config.fish`
   - Zsh: `${ZDOTDIR:-~}/.zshrc`
   - Bash: `~/.bashrc` (created if missing). If your login file (the first of `~/.bash_profile`, `~/.bash_login`, `~/.profile`) does not already load `~/.bashrc` with an uncommented `.` or `source` line, a small loader block is added to it; if none exists, `~/.profile` is created
5. **Starts the agent** (if not already running)

### What the checksums do and do not prove

The digests ship in the same release as the files, so they catch a corrupted
download but not a compromised release. The independent check is the
build-provenance attestation, which the installer does not run for you; with
the [GitHub CLI](https://cli.github.com/), verify the installer before running it:

```bash
curl -fsSLO https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh
gh attestation verify install-oneline.sh --repo jpease/gpy
sh install-oneline.sh
```

The full trust model is in [SECURITY.md](../SECURITY.md#trust-model); other
files are covered in [Verifying Your Download](#verifying-your-download).

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
  curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | GPY_SHELL=fish sh
  ```

- **`GPY_VERSION`**: Install a specific release with whichever copy of the
  installer you ran. The release must publish `gpy-shell-files.tar.gz` (see
  above); the release's own installer URL is the simpler way to pin.
  ```bash
  curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | GPY_VERSION=vX.Y.Z sh
  ```

- **`GPY_NERD_FONT`**: Force the icon style instead of auto-detecting (see
  [Icons and Nerd Fonts](#icons-and-nerd-fonts)). Accepts `1`/`nerd`, `0`/`none`,
  or `unknown`.
  ```bash
  curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | GPY_NERD_FONT=none sh
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
using a Nerd Font, those glyphs render as tofu (`□`) or `?` — so `gpy setup`
picks an icon style that renders on your machine before your first prompt:

- **A Nerd Font is detected** → icons stay on (`ui.show_icons = true`). When a
  terminal is attached, GPY also shows a sample line and asks you to confirm the
  glyphs actually render (font presence alone doesn't prove your terminal is
  configured to use it).
- **No Nerd Font is detected, or detection is inconclusive** → GPY defaults to
  **ASCII** icons (`ui.show_icons = false`), which render on any terminal, and
  prints how to enable the full set later.

`gpy setup` never overwrites an existing `ui.show_icons` value, so upgrades and
already-configured users keep their settings. To re-run detection and
overwrite it, use `gpy setup --redetect-icons`. Or set the style directly:

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

Tab-completion for the `gpy` CLI is installed automatically as part of the standard installation flows. Completions are regenerated on every install or upgrade to match your installed binary.

Completion types:
- `gpy <TAB>` — Subcommands and flags
- `gpy theme use <TAB>`, `gpy theme validate <TAB>` — Installed theme names
- `gpy palette use <TAB>`, `gpy palette validate <TAB>` — Installed palette names
- `gpy enable <TAB>`, `gpy disable <TAB>` — Available segment names

## Homebrew

GPY ships through a personal tap, `jpease/homebrew-tap` (not homebrew-core):

```bash
brew install jpease/tap/gpy
```

On Apple Silicon Macs running macOS 15 or later, and on Linux x86_64, Homebrew
installs a prebuilt bottle. Anywhere else, Intel Macs included, the formula
builds `gpy` and `gpy-agent` from the tagged source tarball, which pulls in
Rust as a build dependency (the curl installer has prebuilt Intel binaries).
Either way it installs the Fish, Zsh and Bash files and the bundled `username`
and `hostname` plugins under Homebrew's `share/gpy/{fish,zsh,bash,plugins}`;
`gpy setup` copies the plugins to `~/.config/gpy/bundled-plugins`.
It depends on none of the three shells and does not modify your dotfiles.
Finish with:

```bash
gpy setup
```

then restart your shell and run `gpy doctor`. See [`gpy setup`](#gpy-setup).

The formula lives in the tap, not in this repository.

## `gpy setup`

`gpy setup` is the shell-wiring step of every install, as a command. The curl
installer does it for you; Homebrew and source builds leave it to you, and it is
the same for Fish, Zsh and Bash:

```bash
gpy setup                    # your login shell ($SHELL)
gpy setup --shell zsh        # a specific shell
gpy setup --from <dir>       # script trees and bundled plugins from a checkout (fish/, zsh/, bash/, plugins/)
```

It:

1. Copies the shell's files to the location the curl installer uses (Fish:
   `~/.config/fish/gpy/`; Zsh and Bash: `~/.config/gpy/<shell>/`), so the two
   installers are interchangeable and `scripts/uninstall.*` removes either.
2. Writes the `gpy` tab completions, generated from the binary you run.
3. Adds one `# >>> gpy-init >>>` block to your rc file (`config.fish`,
   `${ZDOTDIR:-~}/.zshrc`, or `~/.bashrc` plus a login-shell loader), only if
   it is not already there. For Fish it also links `fish_prompt`, moving any
   existing one aside.
4. Picks an icon style for a fresh install (see [Icons and Nerd Fonts](#icons-and-nerd-fonts)).

It is safe to re-run, and after an upgrade it refreshes the copied files: those
of the shell it sets up, and those of every other shell set up earlier (their
rc files are left alone). `gpy doctor` warns about a shell whose files an older
`gpy` installed. It finds the script trees via `--from`, then `GPY_SHARE_DIR`,
then the `share/gpy` beside the binary (Homebrew), then the source checkout the
binary was built in. Given `--from` or `GPY_SHARE_DIR`, as the installers do,
it also keeps a copy of the trees in that `share/gpy` (`~/.local/share/gpy` for
the installers), so a later bare `gpy setup` works. Nothing in an rc file is
touched until the files are in place.

## Verifying Your Download

Every release publishes a `.sha256` sidecar next to each downloadable file, an
aggregate `SHA256SUMS` manifest, and a CycloneDX SBOM (`gpy-sbom.cdx.json`).
The installers verify checksums for you; these steps are for verifying a
manual download, or for confirming what an installer did.

**Checksum:**

```bash
VERSION=v0.7.1   # the release you downloaded
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
VERSION=v0.7.1
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

`gpy setup --from /tmp/gpy` does everything below for you
(see [`gpy setup`](#gpy-setup)); the manual steps are for those who want to see
or change each one.

**For Fish:**
```bash
mkdir -p ~/.config/fish/gpy
cp -r /tmp/gpy/fish/* ~/.config/fish/gpy/

# Add to config.fish. Source conf.d/gpy_init.fish, the same fail-safe
# entry point the installers use (it checks the layout before loading and
# disables GPY cleanly when something is missing); the marker lines are
# what scripts/uninstall.sh removes.
echo '
# >>> gpy-init >>>
# GPY shell integration
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
printf '\n# >>> gpy-init >>>\n# GPY shell integration\nsource ~/.config/gpy/zsh/gpy.zsh\n# <<< gpy-init <<<\n' >> "${ZDOTDIR:-$HOME}/.zshrc"
```

**For Bash:**
```bash
mkdir -p ~/.config/gpy/bash
cp -r /tmp/gpy/bash/* ~/.config/gpy/bash/

# Add to .bashrc (the marker lines are what scripts/uninstall.sh removes)
printf '\n# >>> gpy-init >>>\n# GPY shell integration\nsource ~/.config/gpy/bash/gpy.bash\n# <<< gpy-init <<<\n' >> ~/.bashrc
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

- **Rust**: 1.99 or later — the `rust-version` declared in `gpy-agent/Cargo.toml`, which is what `cargo build` enforces
- **Git**: 2.0 or later
- **Shell**: see [Supported Platforms and Shells](#supported-platforms-and-shells)

### Build Steps

```bash
# Clone repository
git clone https://github.com/jpease/gpy.git
cd gpy

# Build the agent. Use the release-dist profile: `release` is opt-level 3 with
# fat LTO and keeps symbols; release-dist matches the shipped binaries.
(cd gpy-agent && cargo build --profile release-dist)

# The binaries are now in gpy-agent/target/release-dist/
```

### Install Locally

`install.sh` is the release-archive installer: it verifies and installs the
checksummed `bin/` payload a release tarball carries, which a checkout does
not contain, so it cannot be used from source. Install the binaries you built,
then let [`gpy setup`](#gpy-setup) wire up your shell. This works the same for
Fish, Zsh and Bash:

```bash
# Install both binaries (the agent daemon and the gpy CLI)
mkdir -p ~/.local/bin
install -m 755 gpy-agent/target/release-dist/gpy-agent gpy-agent/target/release-dist/gpy ~/.local/bin/

# Wire up your login shell; add --shell fish|zsh|bash to choose another
~/.local/bin/gpy setup --from .
```

`--from .` points setup at this checkout's `fish/`, `zsh/` and `bash/` trees.
(`install-dev.fish` is the contributor workflow: it also rebuilds and verifies
hot reload, and needs Fish.)

Make sure `~/.local/bin` is on your `PATH` (see
[Troubleshooting](#agent-binary-not-found)).

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
- **Socket already in use**: Another agent instance is running, check with `gpy-agent status`

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

macOS ships with Bash 3.2 (from 2007). GPY runs on it, but the duration segment is disabled there. Bash 5.0+ is recommended:

```bash
# Install modern Bash via Homebrew
brew install bash

# Verify version
bash --version
```

## Uninstallation

Run the uninstall script (`sh scripts/uninstall.sh`); it removes the Fish, Zsh and Bash integration alike.
Uninstallation is **global for the current user's install**: the script stops
the agent and supervisor processes, removes the binaries and their upgrade backups
from `~/.local/bin`, removes all shell integration files and completions (Fish,
Zsh, Bash), cleans the `gpy-init` block from all supported startup files
(`~/.bashrc`, `~/.bash_profile`, `~/.bash_login`, `~/.profile`, `${ZDOTDIR:-~}/.zshrc`, `~/.zshrc`, and `~/.config/fish/config.fish`), and
removes the config, cache, and runtime directories. It asks for confirmation first
and removes nothing unless it gets it: with no terminal to answer (stdin closed or
at end of input) it exits non-zero, so pass `--yes` to uninstall from a script.
Startup files are restored byte-for-byte (startup files the installer created are removed).

```bash
# From a release archive or a clone of the repository
sh scripts/uninstall.sh                     # POSIX shell entry point (uninstalls globally)
sh scripts/uninstall.sh --yes               # no prompt (scripts, CI, no terminal)

# Installed with the one-liner and no local copy? Download the script from the
# release. Piping it straight into a shell would feed the confirmation prompt
# the script itself:
curl -fsSL https://github.com/jpease/gpy/releases/latest/download/uninstall.sh -o /tmp/gpy-uninstall.sh
sh /tmp/gpy-uninstall.sh
```

### Fish Prompt Restoration

If your `fish_prompt.fish` is an unrelated custom prompt or symlink, the uninstaller
preserves it byte-for-byte and leaves all backups untouched. The uninstaller removes
only GPY's prompt symlink or a regular-file copy of GPY's own prompt (one that runs
GPY's `for segment in $__enabled_segments` dispatch); a prompt that merely mentions
GPY is yours and stays.

When the prompt destination is absent (or becomes absent after removing the GPY
symlink/implementation), the uninstaller restores the newest backup by comparing the
timestamp suffix (`YYYYMMDD_HHMMSS`) across all accepted naming conventions:
- `fish_prompt.fish.backup.<stamp>` (from `install.sh`, `install-oneline.sh`, `gpy setup` and `install-dev.fish`)
- `fish_prompt.fish.gpy-backup.<stamp>` (from `install.sh` in releases before `gpy setup`)
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

- GPY runs in WSL (Windows Subsystem for Linux); WSL 2 with a distribution such as Ubuntu is what the CI smoke run uses
- Use the Linux installation instructions within WSL

What differs from a native Linux install, and what to do about it:

| Item | Behaviour under WSL | What to do |
|---|---|---|
| Where the repositories live | A repository under `/mnt/c` (or any `/mnt/<drive>`) is on the Windows filesystem, reached over drvfs/9p. `git` is slower there, and on a mount without the `metadata` option an ordinary user cannot create a repository there at all (`git init` fails with `chmod ... Operation not permitted`, observed on the CI runner) and `inotify` may not report changes, so live updates rely on the agent's watcher probe falling back to polling, the periodic reconcile (every 45 s) and the repaint on each command. | Keep repositories in the Linux filesystem (`~/src/...`); if you must use a Windows drive, mount it with `options = "metadata"` under `[automount]` in `/etc/wsl.conf`. For one you must keep on a Windows drive, either set `git.skip_paths = ["/mnt/c"]` to drop git detection there, or start the agent with `GPY_WATCH_FORCE_POLL=1` to poll without waiting for the probe (see [troubleshooting](user/troubleshooting.md#wsl)). |
| `$HOME`, cache and runtime directories | GPY serialises agent start with a file lock and keeps its socket in the runtime directory. Neither is tested on a drvfs or network `$HOME`. | Keep `$HOME` (WSL's default `/home/<user>`) and `XDG_RUNTIME_DIR`/`XDG_CACHE_HOME` on the Linux filesystem. |
| Windows programs on `PATH` | WSL's interop appends the Windows `PATH` by default. The agent runs language tools by bare name (`node`, `python3`, `mise`, ...), never with `.exe`, so a Windows install is reached only if it ships an extensionless file of that name. This has not been traced on a real machine. | If the language segment shows a Windows tool's version, or is slow, set `[interop]` `appendWindowsPath = false` in `/etc/wsl.conf` and restart WSL (`wsl --shutdown`). |
| Nerd Font detection | The terminal that draws the glyphs is a Windows application, so the Linux font directories and `fc-list` say nothing about it. `gpy setup` reports the fonts as undetectable and defaults to ASCII icons. | Install a Nerd Font on Windows and select it in your terminal, then `gpy config set ui.show_icons true`; or set `GPY_NERD_FONT=nerd` (or `none`) before running `gpy setup --redetect-icons`. |
| `gpy config open` | Uses `$VISUAL`, then `$EDITOR`, then `wslview` (from the `wslu` package) when it is installed, then `xdg-open`, which is usually missing on WSL. | Set `$EDITOR` (for example `export EDITOR=nano`), or `sudo apt install wslu`. |

- Native Windows (PowerShell) is not supported for the prompt integration or the agent (IPC is Unix-socket only). Only the CLI builds and runs there; see the operating-systems table above.

## Support

For issues, questions, or feature requests:

- **GitHub Issues**: https://github.com/jpease/gpy/issues
- **Documentation**: https://github.com/jpease/gpy/tree/main/docs
- **Discussions**: https://github.com/jpease/gpy/discussions
