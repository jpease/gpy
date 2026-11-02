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
- [Platform-Specific Notes](#platform-specific-notes)
- [Support](#support)

## Supported Platforms and Shells

This is the canonical statement of what GPY supports. Every other document
(README, release notes, CLI help, Cargo metadata) points here rather than
restating it.

### Operating systems

| OS | Support |
|----|---------|
| Linux (x86_64, aarch64) | Supported on x86_64; aarch64 binaries are built but not run in CI. See [Linux](#linux) |
| macOS 11+ (Intel and Apple Silicon) | Supported on Apple Silicon; Intel binaries are built but not run in CI |
| Windows via WSL | Supported; see [Windows (WSL)](#windows-wsl) |
| Windows native | `gpy` CLI only; the prompt needs Unix sockets |

What CI runs on each platform is in [CI coverage by platform](dev/testing.md#ci-coverage-by-platform).

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
4. **Runs `gpy setup`**, which installs the shell files and the bundled plugins
   under `$XDG_DATA_HOME/gpy` (default `~/.local/share/gpy`) as `fish/`, `zsh/`,
   `bash/` and `plugins/` (the binaries go to `~/.local/bin/gpy-agent` and
   `~/.local/bin/gpy`):

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

The installers take no options other than `--help`; an unknown argument exits 2
before anything is installed. Configure them with these variables (pipe the
one-line installer to `sh`; run `install.sh` from an extracted archive with
`bash install.sh`, since it needs bash). With an unsupported shell (anything but
fish, zsh or bash, or no `$SHELL`) they install the binaries only, edit no shell
files, and ask you to re-run with `GPY_SHELL=fish`, `zsh` or `bash`.

- **`GPY_SHELL`**: Override shell detection
  ```bash
  curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | GPY_SHELL=fish sh
  ```

- **`GPY_VERSION`** (one-line installer only; `install.sh` installs the archive
  it came from): Install a specific release with whichever copy of the
  installer you ran. The release must publish `gpy-shell-files.tar.gz` (see
  above); the release's own installer URL is the simpler way to pin.
  ```bash
  curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | GPY_VERSION=vX.Y.Z sh
  ```

- **`GPY_NERD_FONT`**: Force the icon style instead of detecting it (see
  [Icons and Nerd Fonts](#icons-and-nerd-fonts)). Accepted values:
  [Environment Variables](user/environment-variables.md#setup).
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
(accepted values: [Environment Variables](user/environment-variables.md#setup)) — useful for provisioning scripts and
non-interactive installs. A set `GPY_NERD_FONT` is taken as your answer: `gpy setup` writes `show_icons` from it and does not ask,
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
`gpy setup` copies them to `$XDG_DATA_HOME/gpy` (default `~/.local/share/gpy`).
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

1. Copies the release layout, the `fish/`, `zsh/` and `bash/` trees and
   `plugins/`, to the data root, `$XDG_DATA_HOME/gpy` (default
   `~/.local/share/gpy`; only an absolute `XDG_DATA_HOME` counts), the location
   the curl installer uses, so the two installers are interchangeable and
   `gpy uninstall` removes either. Your `config.toml`, themes, palettes and
   plugins stay in `$XDG_CONFIG_HOME/gpy`.
2. Writes the `gpy` tab completions, generated from the binary you run, for the
   shell you chose and for every other shell whose rc file already has the GPY
   block.
3. Adds one `# >>> gpy-init >>>` block to your rc file (`config.fish`,
   `${ZDOTDIR:-~}/.zshrc`, or `~/.bashrc` plus a login-shell loader), only if
   it is not already there. For Fish it also links `fish_prompt`, moving any
   existing one aside. The Bash block loads GPY only in interactive shells, and
   setup rewrites an unedited Bash block or loader from an earlier release to
   that form; a block whose text differs is left as it is.
4. Moves a theme file in `~/.config/gpy/themes/` that is an unedited copy of a
   builtin theme as released up to 0.7.1 (`default.toml`, `text.toml`,
   `starship.toml`, byte for byte) to `<name>.toml.bak`, so the builtin and its
   later fixes apply again, and says so. Edited copies and symlinks are kept and
   listed on one line. `ui.retire_unmodified_themes = false` turns this off.
5. Picks an icon style for a fresh install (see [Icons and Nerd Fonts](#icons-and-nerd-fonts)).

It is safe to re-run, and after an upgrade it refreshes every tree in the data
root, but wires only the shell you chose (the other shells' rc files are left
alone). `gpy doctor` lists the shells whose rc file has the GPY block and warns
about one whose files an older `gpy` installed. It finds the script trees via
`--from`, then `GPY_SHARE_DIR`, then the `share/gpy` beside the binary
(Homebrew), then the source checkout the binary was built in, and last the data
root itself, so a later bare `gpy setup` works for installer users. When the
source is the data root (the installers' `~/.local/bin/gpy` finds
`~/.local/share/gpy` as its `share/gpy`), nothing is copied: setup still writes
the completions and version stamps and wires the rc file. Nothing in an rc file
is touched until the files are in place.

## Verifying Your Download

Every release publishes a `.sha256` sidecar next to each downloadable file, an
aggregate `SHA256SUMS` manifest, and a CycloneDX SBOM (`gpy-sbom.cdx.json`).
The installers verify checksums for you; these steps are for verifying a
manual download, or for confirming what an installer did.

**Checksum:**

```bash
VERSION=vX.Y.Z   # the release tag you are installing
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

For more control over the installation process, install from the release
assets and let [`gpy setup`](#gpy-setup) do the shell wiring.

### 1. Download and verify the release assets

Download these assets from the [releases page](https://github.com/jpease/gpy/releases),
each with its `.sha256` sidecar: `gpy-agent-<platform>`, `gpy-<platform>` and
`gpy-shell-files.tar.gz`. `<platform>` is `linux-x86_64`, `linux-aarch64`,
`macos-aarch64` (Apple Silicon) or `macos-x86_64` (Intel).

```bash
VERSION=vX.Y.Z   # the release tag you are installing
PLATFORM=linux-x86_64
BASE=https://github.com/jpease/gpy/releases/download/$VERSION

for f in "gpy-agent-$PLATFORM" "gpy-$PLATFORM" gpy-shell-files.tar.gz; do
    curl -fsSLO "$BASE/$f" -O "$BASE/$f.sha256"
    sha256sum -c "$f.sha256"     # macOS: shasum -a 256 -c "$f.sha256"
done
```

Do not skip the checksum step. It is the only thing standing between a
corrupted or substituted download and a binary that runs on every prompt.

### 2. Install the binaries and run `gpy setup`

```bash
mkdir -p ~/.local/bin
install -m 755 "gpy-agent-$PLATFORM" ~/.local/bin/gpy-agent
install -m 755 "gpy-$PLATFORM" ~/.local/bin/gpy

# Unpack the shell files, then let gpy setup install them and wire your shell
SHARE=$(mktemp -d)
tar -xzf gpy-shell-files.tar.gz -C "$SHARE"
~/.local/bin/gpy setup --from "$SHARE"
```

Add `--shell fish|zsh|bash` to wire a shell other than your login shell.
Make sure `~/.local/bin` is on your `PATH` (see
[Troubleshooting](#agent-binary-not-found)).

### 3. Verify the installation

Restart your shell, then run:

```bash
gpy doctor
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
(`just install-dev` is the contributor workflow for Fish users: it also rebuilds and verifies
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
- **Socket permission errors**: Delete the socket `gpy status` reports (`socket: ...`, normally `$XDG_RUNTIME_DIR/gpy/gpy.sock`, else `~/.cache/gpy/gpy.sock`) and retry
- **Socket already in use**: Another agent instance is running, check with `gpy-agent status`

### Prompt not updating

1. Check if agent is running:
   ```bash
   gpy-agent status
   ```

2. Restart the agent:
   ```bash
   gpy-agent restart
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
xattr -d com.apple.quarantine ~/.local/bin/gpy-agent ~/.local/bin/gpy

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

Run `gpy uninstall`; it removes the Fish, Zsh and Bash integration alike.
Uninstallation is **global for the current user's install**: it stops the agent,
removes all shell integration files and completions (Fish,
Zsh, Bash), cleans the `gpy-init` block from all supported startup files
(`~/.bashrc`, `~/.bash_profile`, `~/.bash_login`, `~/.profile`, `${ZDOTDIR:-~}/.zshrc`, `~/.zshrc`, and `~/.config/fish/config.fish`), and
removes the cache and runtime directories. Startup files are restored
byte-for-byte (a startup file `gpy setup` created is removed once it is empty).
Your configuration directory (`~/.config/gpy`: `config.toml`, themes, palettes,
plugins) is kept unless you pass `--purge-config`; without it, the directory is removed
only when nothing is left in it.

It asks for confirmation first and removes nothing unless it gets it: with no
terminal to answer (stdin closed or at end of input) it exits 1, so pass `--yes`
to uninstall from a script. A piped Enter is not a confirmation.

```bash
gpy uninstall                    # asks first; keeps ~/.config/gpy
gpy uninstall --yes              # no prompt (scripts, CI, no terminal)
gpy uninstall --yes --purge-config  # also delete ~/.config/gpy (config, themes, palettes, plugins)
```

The binaries are removed only when the `gpy` you run lives in `~/.local/bin`, the one
directory the installers write (it also removes their `*.backup.*` copies and
`~/.local/share/gpy`). Anywhere else, the binaries and the `share/gpy` beside them
belong to whatever installed them, and `gpy uninstall` leaves them alone and says so.

**Homebrew:** run `gpy uninstall` first, then `brew uninstall gpy`. `gpy uninstall`
cleans your home directory (shell integration, rc blocks, cache); Homebrew removes
`$(brew --prefix)/bin/gpy` and its `share/gpy`.

Releases up to 0.8.0 have no `gpy uninstall`: upgrade first, or run the `uninstall.sh` attached to the release you installed (`https://github.com/jpease/gpy/releases/download/<tag>/uninstall.sh`); `releases/latest` no longer publishes it.

### Fish Prompt Restoration

If your `fish_prompt.fish` is an unrelated custom prompt or symlink, `gpy uninstall`
preserves it byte-for-byte and leaves all backups untouched. It removes
only GPY's prompt symlink or a regular-file copy of GPY's own prompt (one that runs
GPY's `for segment in $__enabled_segments` dispatch); a prompt that merely mentions
GPY is yours and stays.

When the prompt destination is absent (or becomes absent after removing the GPY
symlink/implementation), `gpy uninstall` restores the newest
`fish_prompt.fish.backup.<stamp>` (`YYYYMMDD_HHMMSS`, written by `install.sh`,
`install-oneline.sh`, `gpy setup` and `install-dev.fish`). Older backups remain
untouched. Backups from older releases, named `fish_prompt.fish.gpy-backup.<stamp>`
or `fish_prompt.fish.backup-<stamp>`, are not restored automatically; rename the one
you want to `fish_prompt.fish`.

Every installer moves an existing prompt that is not GPY's own link aside with `mv`,
so a symlinked prompt (stow, yadm, chezmoi) stays a symlink to the same dotfile and is
restored as one. A backup that is a symlink into GPY's own prompt directory is never
restored.

### Manual Removal Checklist

To remove GPY manually instead, this is everything `gpy uninstall` touches:

```bash
gpy-agent stop
rm -f ~/.local/bin/gpy-agent ~/.local/bin/gpy ~/.local/bin/gpy-agent.backup.* ~/.local/bin/gpy.backup.*
rm -rf ~/.config/fish/conf.d/gpy_init.fish ~/.config/fish/completions/gpy.fish ~/.config/fish/completions/gpy-dynamic.fish
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/gpy"   # shell files, completions and bundled plugins
# rm -rf ~/.config/gpy  # your config, themes, palettes and plugins are kept unless you delete them
rm -rf ~/.cache/gpy "${XDG_RUNTIME_DIR:-~/.cache}/gpy"          # cache; socket, shells/
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
