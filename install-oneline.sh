#!/bin/sh
# GPY One-Line Installer
# Usage: curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | sh

set -e

# Colors and formatting
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    RESET=''
fi

# Logging helpers
info() { printf "${BLUE}ℹ${RESET} %s\n" "$1"; }
success() { printf "${GREEN}✓${RESET} %s\n" "$1"; }
warn() { printf "${YELLOW}⚠${RESET} %s\n" "$1"; }
error() { printf "${RED}✗${RESET} %s\n" "$1" >&2; }
bold() { printf "${BOLD}%s${RESET}\n" "$1"; }

# Error handler
die() {
    error "$1"
    exit 1
}

# --- download and verification helpers ----------------------------------------
#
# The downloader is resolved once here instead of branching on curl-vs-wget at
# every call site; #494 replaced four near-identical copies of that branch,
# each of which had to be kept in step with the others by hand.

if command -v curl >/dev/null 2>&1; then
    DOWNLOADER=curl
elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER=wget
else
    DOWNLOADER=""
fi

# Fetch $1 into the file at $2. Non-zero on any HTTP or transport error.
fetch_to() {
    case "$DOWNLOADER" in
        curl) curl -fsSL "$1" -o "$2" 2>/dev/null ;;
        wget) wget -q "$1" -O "$2" 2>/dev/null ;;
        *) return 1 ;;
    esac
}

# Fetch $1 to stdout. Non-zero on any HTTP or transport error.
fetch_stdout() {
    case "$DOWNLOADER" in
        curl) curl -fsSL "$1" 2>/dev/null ;;
        wget) wget -qO- "$1" 2>/dev/null ;;
        *) return 1 ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

# Verify a downloaded file against the .sha256 sidecar the release publishes
# beside it (scripts/package-release.sh writes one per asset).
#
# Every branch fails closed. Before #494 this was opportunistic: a release with
# no sidecar -- which was every release -- silently skipped verification, so an
# attacker who could serve a binary could also serve no checksum and the
# installer would shrug and continue. A missing sidecar and a stripped one are
# indistinguishable from here, so both abort.
verify_download() {
    verify_file="$1"
    verify_url="$2"
    verify_label="$3"

    verify_expected="$(fetch_stdout "$verify_url.sha256" | awk '{print $1}' || true)"

    if [ -z "$verify_expected" ]; then
        die "No published checksum for $verify_label ($verify_url.sha256). Refusing to install an unverified binary."
    fi

    verify_actual="$(sha256_of "$verify_file")" ||
        die "Could not compute a SHA-256 checksum for $verify_label"

    if [ "$verify_actual" != "$verify_expected" ]; then
        die "Checksum mismatch for $verify_label (expected $verify_expected, got $verify_actual). Refusing to install."
    fi

    success "Verified $verify_label checksum"
}

# Drop the macOS quarantine flag so Gatekeeper does not SIGKILL an unsigned
# binary. Only ever called after verify_download has matched the file against
# its published digest, so it never lowers a barrier on an unverified download.
#
# Scoped to com.apple.quarantine rather than the previous `xattr -c`, which
# stripped every extended attribute including provenance metadata (#494).
clear_quarantine() {
    if [ "$PLATFORM" = "darwin" ] && command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$1" 2>/dev/null || true
    fi
}

main() {
# Header
echo ""
bold "═══════════════════════════════════════"
bold "  GPY: Guppy Prompt, Yay! 🐠"
bold "  One-Line Installer"
bold "═══════════════════════════════════════"
echo ""

# Both requirements are fatal up front rather than at the point of use: there
# is no partial install worth starting if the machine cannot download or cannot
# verify what it downloads (#494).
if [ -z "$DOWNLOADER" ]; then
    die "Neither curl nor wget found. Please install one and try again."
fi

if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    die "No SHA-256 tool found (need sha256sum or shasum). GPY verifies every download before installing it."
fi

# Detect platform and architecture
info "Detecting system configuration..."
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Each platform/arch resolves BOTH the agent daemon ($BINARY_NAME) and the
# user-facing CLI ($CLI_BINARY_NAME). They ship as a parallel pair of release
# assets (gpy-agent-<platform>-<arch> and gpy-<platform>-<arch>); the CLI
# powers shell completions and `gpy` subcommands (#327).
#
# No mingw*/msys*/cygwin* arm here, unlike install.sh's table: this installer
# sets up shell integration (Fish/Zsh/Bash config, conf.d, completions),
# which is Unix-socket-only, so there is nothing for it to install on
# Windows even where a binary exists (#615).
case "$PLATFORM" in
    linux)
        case "$ARCH" in
            x86_64) BINARY_NAME="gpy-agent-linux-x86_64"; CLI_BINARY_NAME="gpy-linux-x86_64" ;;
            aarch64|arm64) BINARY_NAME="gpy-agent-linux-aarch64"; CLI_BINARY_NAME="gpy-linux-aarch64" ;;
            *) die "Unsupported architecture: $ARCH (supported: x86_64, aarch64)" ;;
        esac
        ;;
    darwin)
        case "$ARCH" in
            x86_64) BINARY_NAME="gpy-agent-macos-x86_64"; CLI_BINARY_NAME="gpy-macos-x86_64" ;;
            arm64) BINARY_NAME="gpy-agent-macos-aarch64"; CLI_BINARY_NAME="gpy-macos-aarch64" ;;
            *) die "Unsupported architecture: $ARCH (supported: x86_64, arm64)" ;;
        esac
        ;;
    *)
        error "Unsupported platform: $PLATFORM (supported: linux, darwin/macOS)"
        echo ""
        info "Build from source:"
        echo "   https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
        exit 1
        ;;
esac

success "Platform: $PLATFORM ($ARCH)"

# Detect shell
info "Detecting shell..."
CURRENT_SHELL="${SHELL##*/}"

# Allow override via environment variable
if [ -n "$GPY_SHELL" ]; then
    CURRENT_SHELL="$GPY_SHELL"
    info "Using shell from GPY_SHELL environment variable: $CURRENT_SHELL"
fi

case "$CURRENT_SHELL" in
    fish|zsh|bash)
        success "Detected shell: $CURRENT_SHELL"
        ;;
    *)
        warn "Unknown shell: $CURRENT_SHELL, defaulting to bash"
        CURRENT_SHELL="bash"
        ;;
esac

# Set up directories
INSTALL_DIR="$HOME/.local/bin"

# Fish honors XDG_CONFIG_HOME for its own config dir; hardcoding
# $HOME/.config here would install to a directory fish never reads for users
# who've set it (#324, matching install.sh).
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"

# Fish uses $CONFIG_HOME/fish/gpy/ to match existing install.sh
# Other shells use $CONFIG_HOME/gpy/{zsh,bash}/
if [ "$CURRENT_SHELL" = "fish" ]; then
    SHELL_CONFIG_DIR="$CONFIG_HOME/fish/gpy"
else
    SHELL_CONFIG_DIR="$CONFIG_HOME/gpy/$CURRENT_SHELL"
fi

# GitHub release information
REPO="jpease/gpy"
VERSION="${GPY_VERSION:-latest}"

# Resolve the release tag. This used to fall back to VERSION="main" whenever
# the GitHub API call failed, which pointed both the binary download and the
# shell-file download at a mutable branch -- an unpinned, unverifiable install
# whose contents changed with every push. A release install now either resolves
# a tag or stops (#494).
if [ "$VERSION" = "latest" ]; then
    info "Fetching latest release information..."
    LATEST_TAG="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" |
        grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
    if [ -z "$LATEST_TAG" ]; then
        die "Could not resolve the latest GPY release from the GitHub API. Check your network, or pin a version: GPY_VERSION=v1.2.3 ... | sh"
    fi
    VERSION="$LATEST_TAG"
    success "Latest version: $VERSION"
fi

# Install agent binary
info "Installing GPY agent..."

mkdir -p "$INSTALL_DIR"

# Download binary. Fixed /tmp/gpy-agent.$$ paths were both predictable
# (guessable/pre-creatable by another local user) and never cleaned up on a
# `die` exit (partial downloads leaked in /tmp); mktemp plus the trap below
# fix both (#324).
BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY_NAME"
TEMP_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-agent.XXXXXX") || die "Failed to create a temp file for the download"
trap 'rm -f "$TEMP_BINARY" 2>/dev/null' EXIT

# A failed download used to fall back to
# raw.githubusercontent.com/$REPO/main/bin/$BINARY_NAME. That path has never
# existed -- there is no tracked bin/ directory -- so the fallback only turned
# a clear "release asset missing" into a confusing second 404, while
# advertising a mutable, unverifiable install route. Removed in #494.
info "Downloading agent from $BINARY_URL..."
if ! fetch_to "$BINARY_URL" "$TEMP_BINARY"; then
    die "Failed to download $BINARY_NAME from the $VERSION release. Check your internet connection, or see https://github.com/$REPO/releases"
fi

verify_download "$TEMP_BINARY" "$BINARY_URL" "$BINARY_NAME"

# Back up only once there is a verified replacement to install. Backing up
# first meant a failed download or a failed verification still littered
# ~/.local/bin with a backup copy of a binary that was never replaced (#494).
if [ -f "$INSTALL_DIR/gpy-agent" ]; then
    BACKUP_PATH="$INSTALL_DIR/gpy-agent.backup.$(date +%Y%m%d_%H%M%S)"
    info "Backing up existing agent to $BACKUP_PATH"
    cp "$INSTALL_DIR/gpy-agent" "$BACKUP_PATH"
fi

# Install binary
chmod +x "$TEMP_BINARY"
mv "$TEMP_BINARY" "$INSTALL_DIR/gpy-agent"
clear_quarantine "$INSTALL_DIR/gpy-agent"

# Verify binary works
if ! "$INSTALL_DIR/gpy-agent" --version >/dev/null 2>&1; then
    die "Agent binary installation failed (binary not functional)"
fi

success "Agent binary installed to $INSTALL_DIR/gpy-agent"

# Install the CLI binary (gpy). Unlike the agent, an ABSENT gpy asset is
# non-fatal: the prompt works without it (gpy only powers the CLI and shell
# completions), so a release that is missing it degrades rather than blocking
# every install (#327).
#
# A gpy asset that downloads but fails verification is a different matter and
# aborts the install, same as the agent. Absence is tolerated; tampering is
# not (#494).
info "Installing GPY CLI (gpy)..."

CLI_BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$CLI_BINARY_NAME"
TEMP_CLI_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-cli.XXXXXX") || die "Failed to create a temp file for the CLI download"
trap 'rm -f "$TEMP_BINARY" "$TEMP_CLI_BINARY" 2>/dev/null' EXIT

CLI_DOWNLOADED=0
info "Downloading CLI from $CLI_BINARY_URL..."
if fetch_to "$CLI_BINARY_URL" "$TEMP_CLI_BINARY"; then
    CLI_DOWNLOADED=1
fi

if [ "$CLI_DOWNLOADED" -eq 1 ]; then
    verify_download "$TEMP_CLI_BINARY" "$CLI_BINARY_URL" "$CLI_BINARY_NAME"
    chmod +x "$TEMP_CLI_BINARY"

    if [ -f "$INSTALL_DIR/gpy" ]; then
        CLI_BACKUP_PATH="$INSTALL_DIR/gpy.backup.$(date +%Y%m%d_%H%M%S)"
        info "Backing up existing CLI to $CLI_BACKUP_PATH"
        cp "$INSTALL_DIR/gpy" "$CLI_BACKUP_PATH"
    fi

    # mv (atomic rename) replaces the directory entry rather than overwriting
    # the inode in place, so it's ETXTBSY-safe if an old gpy is mid-execution.
    mv "$TEMP_CLI_BINARY" "$INSTALL_DIR/gpy"
    clear_quarantine "$INSTALL_DIR/gpy"

    if "$INSTALL_DIR/gpy" --version >/dev/null 2>&1; then
        success "CLI binary installed to $INSTALL_DIR/gpy"
    else
        warn "CLI binary installed but not functional; shell completions may be unavailable"
    fi
else
    warn "Could not download the gpy CLI ($CLI_BINARY_NAME); shell completions will be unavailable"
    warn "The prompt itself is unaffected. Re-run this installer after the next release to add gpy."
fi

# Check if ~/.local/bin is in PATH
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *":$INSTALL_DIR:"*) ;;
    *)
        warn "$INSTALL_DIR is not in your PATH"
        warn "Add this to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

# Install shell files
info "Installing $CURRENT_SHELL integration files..."

mkdir -p "$SHELL_CONFIG_DIR"

# Download shell files from GitHub. Pinned to the same $VERSION as the
# binary (a resolved release tag) so a fresh install never pairs a
# release binary with unreleased shell scripts (#308).
SHELL_FILES_BASE_URL="https://raw.githubusercontent.com/$REPO/$VERSION/$CURRENT_SHELL"

# Create temp directory for downloads. mktemp instead of a fixed
# /tmp/gpy-install.$$ path for the same predictability/leak reasons as
# TEMP_BINARY above; the trap replaces the earlier one and cleans up both
# temps together (#324).
TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-install.XXXXXX") || die "Failed to create a temp directory for downloads"
trap 'rm -f "$TEMP_BINARY" "$TEMP_CLI_BINARY" 2>/dev/null; rm -rf "$TEMP_DIR" 2>/dev/null' EXIT

# Per-shell file lists. Kept in named variables (instead of inline in the
# for-loops below) so tests/fish/install_oneline_file_lists.test.fish can
# verify these stay in sync with the files actually shipped in the repo,
# catching list rot like #308 before it reaches users.
FISH_CORE_FILES="constants.fish debug.fish init.fish ipc.fish renderer.fish util.fish"
FISH_SEGMENT_FILES="clock.fish devtools.fish directory.fish duration.fish git.fish hostname.fish language.fish status.fish username.fish"
FISH_FUNCTION_FILES="fish_prompt.fish"

ZSH_CORE_FILES="constants.zsh init.zsh ipc.zsh signals.zsh supervisor.zsh"
ZSH_SEGMENT_FILES="clock.zsh directory.zsh duration.zsh git.zsh hostname.zsh language.zsh status.zsh username.zsh"

BASH_CORE_FILES="constants.bash init.bash ipc.bash signals.bash supervisor.bash"
BASH_SEGMENT_FILES="clock.bash directory.bash duration.bash git.bash hostname.bash language.bash status.bash username.bash"

# Download shell files based on shell type
case "$CURRENT_SHELL" in
    fish)
        info "Downloading Fish shell files..."
        for dir in core segments functions conf.d; do
            mkdir -p "$SHELL_CONFIG_DIR/$dir"
        done

        # Download core files. Core files are required for GPY to function
        # (unlike segments/functions below, which are best-effort) -- a
        # failed download must stop the install before any rc file is
        # touched, so a partial install never leaves rc files pointing at an
        # incomplete shell integration (#309).
        for file in $FISH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        fetch_to "https://raw.githubusercontent.com/$REPO/$VERSION/fish/conf.d/gpy_init.fish" "$SHELL_CONFIG_DIR/conf.d/gpy_init.fish" || warn "Could not download gpy_init.fish"

        # Download segments
        for segment in $FISH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # Download functions
        for func in $FISH_FUNCTION_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/functions/$func" "$SHELL_CONFIG_DIR/functions/$func" || warn "Could not download $func"
        done

        # Install Fish completions. `completions/gpy.fish` is STRUCTURAL
        # (subcommand/flag names) and is regenerated from the installed CLI
        # on every run rather than downloaded, so it can never drift from
        # the binary actually on disk. Skipped -- not fatal -- when the gpy
        # CLI failed to download above ($CLI_DOWNLOADED=0, #327): the prompt
        # itself doesn't depend on shell completions.
        # `completions/gpy-dynamic.fish` is the small, checked-in glue that
        # supplies live theme/palette/segment names via `gpy __complete
        # <kind>` (#328, #329). It's downloaded best-effort like the
        # segment/function files above rather than treated as a required
        # core file: a failed download only loses dynamic-value completions,
        # not the prompt itself.
        FISH_COMPLETIONS_DIR="$CONFIG_HOME/fish/completions"
        mkdir -p "$FISH_COMPLETIONS_DIR"

        if [ -x "$INSTALL_DIR/gpy" ]; then
            if "$INSTALL_DIR/gpy" completions fish >"$FISH_COMPLETIONS_DIR/gpy.fish" 2>/dev/null; then
                success "Generated structural completions: $FISH_COMPLETIONS_DIR/gpy.fish"
            else
                warn "Could not generate gpy completions (gpy completions fish failed)"
            fi
        else
            warn "gpy CLI not installed; skipping shell completions"
        fi

        fetch_to "$SHELL_FILES_BASE_URL/completions/gpy-dynamic.fish" "$FISH_COMPLETIONS_DIR/gpy-dynamic.fish" || warn "Could not download gpy-dynamic.fish"
        ;;

    zsh)
        info "Downloading Zsh shell files..."
        mkdir -p "$SHELL_CONFIG_DIR/core" "$SHELL_CONFIG_DIR/segments"

        # Download gpy.zsh entry point
        fetch_to "$SHELL_FILES_BASE_URL/gpy.zsh" "$SHELL_CONFIG_DIR/gpy.zsh" || die "Failed to download gpy.zsh"

        # Download core files. Core files are required for GPY to function
        # (unlike segments below, which are best-effort) -- a failed
        # download must stop the install before any rc file is touched, so
        # a partial install never leaves rc files pointing at an incomplete
        # shell integration (#309).
        for file in $ZSH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        # Download segments
        for segment in $ZSH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # Install Zsh completions. `completions/_gpy` is STRUCTURAL
        # (subcommand/flag names), regenerated fresh from the CLI binary
        # since it must match the exact clap definitions shipped in this
        # package -- same as the Bash/Fish structural completions above.
        # `completions/_gpy-dynamic` is the checked-in, hand-authored glue
        # that fills in dynamic values (themes, palettes, segments) and
        # delegates to the structural function otherwise; it is downloaded
        # like the other shell files. Both are best-effort: the CLI binary
        # may be absent (#327 -- the CLI powers completions but the prompt
        # itself does not depend on it), and a failed download only loses
        # completions, not the prompt. `zsh/gpy.zsh` wires both into fpath
        # and registers them with `compdef` on every shell start, so no
        # rc-file changes are needed here beyond the gpy-init block already
        # written below.
        mkdir -p "$SHELL_CONFIG_DIR/completions"

        if [ -x "$INSTALL_DIR/gpy" ]; then
            if "$INSTALL_DIR/gpy" completions zsh >"$SHELL_CONFIG_DIR/completions/_gpy" 2>/dev/null; then
                success "Generated structural completions: $SHELL_CONFIG_DIR/completions/_gpy"
            else
                warn "Could not generate gpy completions (gpy completions zsh failed)"
            fi
        else
            warn "gpy CLI not installed; skipping shell completions"
        fi

        fetch_to "$SHELL_FILES_BASE_URL/completions/_gpy-dynamic" "$SHELL_CONFIG_DIR/completions/_gpy-dynamic" || warn "Could not download _gpy-dynamic"
        ;;

    bash)
        info "Downloading Bash shell files..."
        mkdir -p "$SHELL_CONFIG_DIR/core" "$SHELL_CONFIG_DIR/segments"

        # Download gpy.bash entry point
        fetch_to "$SHELL_FILES_BASE_URL/gpy.bash" "$SHELL_CONFIG_DIR/gpy.bash" || die "Failed to download gpy.bash"

        # Download core files. Core files are required for GPY to function
        # (unlike segments below, which are best-effort) -- a failed
        # download must stop the install before any rc file is touched, so
        # a partial install never leaves rc files pointing at an incomplete
        # shell integration (#309).
        for file in $BASH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        # Download segments
        for segment in $BASH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # Install Bash completions. `completions/gpy.bash` is STRUCTURAL
        # (subcommand/flag names), regenerated fresh from the CLI binary
        # since it must match the exact clap definitions shipped in this
        # package. `completions/gpy-dynamic.bash` is the checked-in,
        # hand-authored glue that fills in dynamic values (themes, palettes,
        # segments) and delegates to the structural function otherwise; it
        # is downloaded like the other shell files. Both are best-effort:
        # the CLI binary may be absent (#327 -- the CLI powers completions
        # but the prompt itself does not depend on it), and a failed
        # download only loses completions, not the prompt.
        mkdir -p "$SHELL_CONFIG_DIR/completions"

        if [ -x "$INSTALL_DIR/gpy" ]; then
            if "$INSTALL_DIR/gpy" completions bash >"$SHELL_CONFIG_DIR/completions/gpy.bash" 2>/dev/null; then
                success "Generated structural completions: $SHELL_CONFIG_DIR/completions/gpy.bash"
            else
                warn "Could not generate gpy completions (gpy completions bash failed)"
            fi
        else
            warn "gpy CLI not installed; skipping shell completions"
        fi

        fetch_to "$SHELL_FILES_BASE_URL/completions/gpy-dynamic.bash" "$SHELL_CONFIG_DIR/completions/gpy-dynamic.bash" || warn "Could not download gpy-dynamic.bash"
        ;;
esac

success "Shell files installed to $SHELL_CONFIG_DIR"

# Configure shell RC file
info "Configuring shell initialization..."

case "$CURRENT_SHELL" in
    fish)
        FISH_CONFIG="$CONFIG_HOME/fish/config.fish"
        FISH_FUNCTIONS_DIR="$CONFIG_HOME/fish/functions"

        mkdir -p "$(dirname "$FISH_CONFIG")"
        mkdir -p "$FISH_FUNCTIONS_DIR"

        touch "$FISH_CONFIG"

        # Add GPY initialization to config.fish. Unquoted heredoc so the
        # sourced path matches wherever SHELL_CONFIG_DIR actually landed
        # (CONFIG_HOME, not a hardcoded ~/.config) -- a literal '~' here
        # would silently miss an XDG_CONFIG_HOME install (#615).
        if ! grep -qF "# >>> gpy-init >>>" "$FISH_CONFIG" 2>/dev/null; then
            cat >> "$FISH_CONFIG" << EOF

# >>> gpy-init >>>
# GPY Prompt Enhancement
if status is-interactive
    source $SHELL_CONFIG_DIR/conf.d/gpy_init.fish
end
# <<< gpy-init <<<
EOF
            success "Added GPY initialization to $FISH_CONFIG"
        else
            success "GPY already configured in $FISH_CONFIG"
        fi

        # Create symlink for fish_prompt
        FISH_PROMPT_TARGET="$FISH_FUNCTIONS_DIR/fish_prompt.fish"
        FISH_PROMPT_SOURCE="$SHELL_CONFIG_DIR/functions/fish_prompt.fish"

        if [ -e "$FISH_PROMPT_TARGET" ] && [ ! -L "$FISH_PROMPT_TARGET" ]; then
            BACKUP="$FISH_PROMPT_TARGET.backup.$(date +%Y%m%d_%H%M%S)"
            info "Backing up existing fish_prompt to $BACKUP"
            mv "$FISH_PROMPT_TARGET" "$BACKUP"
        fi

        if [ -L "$FISH_PROMPT_TARGET" ] && [ "$(readlink "$FISH_PROMPT_TARGET")" = "$FISH_PROMPT_SOURCE" ]; then
            success "Fish prompt already linked"
        else
            ln -sf "$FISH_PROMPT_SOURCE" "$FISH_PROMPT_TARGET"
            success "Linked GPY prompt function"
        fi
        ;;

    zsh)
        ZSH_RC="$HOME/.zshrc"
        touch "$ZSH_RC"

        SOURCE_LINE="source $SHELL_CONFIG_DIR/gpy.zsh"
        if ! grep -qF "# >>> gpy-init >>>" "$ZSH_RC" 2>/dev/null; then
            cat >> "$ZSH_RC" << EOF

# >>> gpy-init >>>
# GPY Prompt Enhancement
$SOURCE_LINE
# <<< gpy-init <<<
EOF
            success "Added GPY initialization to $ZSH_RC"
        else
            success "GPY already configured in $ZSH_RC"
        fi
        ;;

    bash)
        # Try .bashrc first, fall back to .bash_profile
        if [ -f "$HOME/.bashrc" ]; then
            BASH_RC="$HOME/.bashrc"
        else
            BASH_RC="$HOME/.bash_profile"
        fi
        touch "$BASH_RC"

        SOURCE_LINE="source $SHELL_CONFIG_DIR/gpy.bash"
        if ! grep -qF "# >>> gpy-init >>>" "$BASH_RC" 2>/dev/null; then
            cat >> "$BASH_RC" << EOF

# >>> gpy-init >>>
# GPY Prompt Enhancement
$SOURCE_LINE
# <<< gpy-init <<<
EOF
            success "Added GPY initialization to $BASH_RC"
        else
            success "GPY already configured in $BASH_RC"
        fi

        # Warn about Bash version if on macOS. Probe the `bash` binary
        # directly rather than $SHELL: GPY_SHELL can force CURRENT_SHELL=bash
        # while the user's actual login shell is something else (e.g. ksh),
        # whose --version output wouldn't match this parse and could abort
        # the whole script under `set -e` -- after the rc file above was
        # already modified (#324).
        if [ "$PLATFORM" = "darwin" ] && command -v bash >/dev/null 2>&1; then
            BASH_MAJOR_VERSION=$(bash --version 2>/dev/null | head -n1 | sed -n 's/.*version \([0-9][0-9]*\).*/\1/p')
            if [ -n "$BASH_MAJOR_VERSION" ] && [ "$BASH_MAJOR_VERSION" -lt 5 ] 2>/dev/null; then
                warn "macOS ships with Bash 3.2 (from 2007)"
                warn "For best GPY experience, upgrade to Bash 5: brew install bash"
            fi
        fi
        ;;
esac

# Choose an icon style that renders on this machine before the first prompt.
# `gpy-agent init` detects whether a Nerd Font is available and, on a fresh
# install only (no existing config), writes the config with a matching
# `show_icons` so a machine without a Nerd Font never renders tofu boxes on
# its very first prompt (#411). It reads the user's confirmation from
# /dev/tty (not this piped stdin), so a `curl | sh` install with an attached
# terminal can still show a sample and ask; a fully non-interactive pipe
# falls back to detection. Non-fatal (`|| warn`) so a failure never aborts
# the install under `set -e`.
info "Configuring prompt icons..."
"$INSTALL_DIR/gpy-agent" init || warn "Icon setup skipped (gpy-agent init failed); using built-in defaults"

# Always run `start`, even if an agent already responds to `status`: `start`
# is idempotent (no-op when a matching version is already running) and is
# the only path that detects and restarts a version-mismatched old daemon
# (#307). Skipping it whenever `status` succeeded left upgrades never
# restarting the old daemon, so new shells kept talking to a stale protocol.
info "Starting GPY agent..."

if "$INSTALL_DIR/gpy-agent" start >/dev/null 2>&1; then
    success "GPY agent started"
else
    warn "Could not start agent automatically (will start on next shell session)"
fi

# Cleanup temp directory
rm -rf "$TEMP_DIR"

# Final success message
echo ""
bold "═══════════════════════════════════════"
success "GPY installed successfully! 🎉"
bold "═══════════════════════════════════════"
echo ""

info "📦 Installation summary:"
echo "   • Agent: $INSTALL_DIR/gpy-agent"
if [ -x "$INSTALL_DIR/gpy" ]; then
    echo "   • CLI:   $INSTALL_DIR/gpy"
fi
echo "   • Shell files: $SHELL_CONFIG_DIR"
case "$CURRENT_SHELL" in
    fish) echo "   • Config: $FISH_CONFIG" ;;
    zsh) echo "   • Config: $HOME/.zshrc" ;;
    bash) echo "   • Config: $BASH_RC" ;;
esac
echo ""

info "🚀 Next steps:"
case "$CURRENT_SHELL" in
    fish)
        echo "   1. Restart Fish: exec fish"
        echo "   2. Or reload config: source $FISH_CONFIG"
        ;;
    zsh)
        echo "   1. Restart Zsh: exec zsh"
        echo "   2. Or reload config: source ~/.zshrc"
        ;;
    bash)
        echo "   1. Restart Bash: exec bash"
        echo "   2. Or reload config: source $BASH_RC"
        ;;
esac
echo ""

info "✨ Your prompt now includes:"
echo "   • Git status indicators (branch, staged/unstaged/untracked)"
echo "   • Language detection (14+ languages)"
echo "   • Real-time updates via filesystem watching"
echo "   • Optimized performance (<40ms even on large repos)"
echo ""

info "🔧 Customize GPY:"
echo "   • View config: cat $CONFIG_HOME/gpy/config.toml"
echo "   • Agent commands: gpy-agent --help"
echo "   • Documentation: https://github.com/jpease/gpy"
echo ""

info "💡 Troubleshooting:"
echo "   • Check agent status: gpy-agent status"
echo "   • View logs: tail -f $CACHE_HOME/gpy/agent.log"
# The uninstall scripts are the only complete removal path (binaries plus
# their upgrade backups, completions, the rc-file block, config, cache and
# runtime dirs, and the running processes). Download-then-run rather than
# `curl | sh`: the script asks for confirmation on stdin, and a piped script
# would feed itself to that read (#642).
UNINSTALL_URL="https://raw.githubusercontent.com/$REPO/$VERSION/scripts"
case $CURRENT_SHELL in
    fish)
        echo "   • Uninstall: curl -fsSL $UNINSTALL_URL/uninstall.fish -o /tmp/gpy-uninstall.fish && fish /tmp/gpy-uninstall.fish"
        ;;
    *)
        echo "   • Uninstall: curl -fsSL $UNINSTALL_URL/uninstall.sh -o /tmp/gpy-uninstall.sh && GPY_SHELL=$CURRENT_SHELL sh /tmp/gpy-uninstall.sh"
        ;;
esac
echo ""

bold "Happy prompting! 🐠"
echo ""
}

main "$@"
