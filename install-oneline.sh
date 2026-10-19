#!/bin/sh
# GPY one-line installer, published as a release asset:
#   curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh
# package-release.sh stamps RELEASE_VERSION; empty (source checkout) means the
# latest release. GPY_VERSION overrides either.

set -e

RELEASE_VERSION=""

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

info() { printf "${BLUE}ℹ${RESET} %s\n" "$1"; }
success() { printf "${GREEN}✓${RESET} %s\n" "$1"; }
warn() { printf "${YELLOW}⚠${RESET} %s\n" "$1"; }
error() { printf "${RED}✗${RESET} %s\n" "$1" >&2; }
bold() { printf "${BOLD}%s${RESET}\n" "$1"; }

die() {
    error "$1"
    exit 1
}

if command -v curl >/dev/null 2>&1; then
    DOWNLOADER=curl
elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER=wget
else
    DOWNLOADER=""
fi

fetch_to() {
    case "$DOWNLOADER" in
        curl) curl -fsSL "$1" -o "$2" 2>/dev/null ;;
        wget) wget -q "$1" -O "$2" 2>/dev/null ;;
        *) return 1 ;;
    esac
}

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

# Fails closed: whoever can serve a binary can serve no checksum, so an
# unreachable or empty sidecar aborts like a mismatch.
verify_download() {
    verify_file="$1"
    verify_url="$2"
    verify_label="$3"

    if ! verify_sidecar="$(fetch_stdout "$verify_url.sha256")"; then
        die "Could not download the published checksum for $verify_label ($verify_url.sha256). Refusing to install an unverified binary."
    fi
    verify_expected="$(printf '%s\n' "$verify_sidecar" | awk '{print $1}')"

    if [ -z "$verify_expected" ]; then
        die "Published checksum for $verify_label ($verify_url.sha256) is empty. Refusing to install an unverified binary."
    fi

    verify_actual="$(sha256_of "$verify_file")" ||
        die "Could not compute a SHA-256 checksum for $verify_label"

    if [ "$verify_actual" != "$verify_expected" ]; then
        die "Checksum mismatch for $verify_label (expected $verify_expected, got $verify_actual). Refusing to install."
    fi

    success "Verified $verify_label checksum"
}

# Gatekeeper SIGKILLs quarantined unsigned binaries. Call only after
# verify_download; scoped so other xattrs survive.
clear_quarantine() {
    if [ "$PLATFORM" = "darwin" ] && command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$1" 2>/dev/null || true
    fi
}

main() {
bold "GPY one-line installer"
echo ""

# Fatal up front: nothing is worth installing if it cannot be downloaded and verified.
if [ -z "$DOWNLOADER" ]; then
    die "Neither curl nor wget found. Please install one and try again."
fi

if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    die "No SHA-256 tool found (need sha256sum or shasum). GPY verifies every download before installing it."
fi

info "Detecting system configuration..."
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# No Windows arm, unlike install.sh: there is no shell integration to install there.
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

info "Detecting shell..."
CURRENT_SHELL="${SHELL##*/}"

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

INSTALL_DIR="$HOME/.local/bin"

# Fish honors XDG_CONFIG_HOME for its own config dir, so the install must too.
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"

if [ "$CURRENT_SHELL" = "fish" ]; then
    SHELL_CONFIG_DIR="$CONFIG_HOME/fish/gpy"
else
    SHELL_CONFIG_DIR="$CONFIG_HOME/gpy/$CURRENT_SHELL"
fi

# The rc block sources this path inside double quotes; refuse characters that
# would break out rather than escape them for three shells.
case "$SHELL_CONFIG_DIR" in
    *[\"\$\`\\]* | *"
"*)
        die "Cannot install: $SHELL_CONFIG_DIR contains a quote, \$, backtick, backslash or newline, which cannot be sourced safely from a shell rc file. Use an XDG_CONFIG_HOME without those characters."
        ;;
esac

REPO="jpease/gpy"
VERSION="${GPY_VERSION:-${RELEASE_VERSION:-latest}}"

# Resolve the release tag or stop: never fall back to a mutable branch, which
# would pair an unpinned, unverifiable binary with unreleased shell files.
if [ "$VERSION" = "latest" ]; then
    info "Fetching latest release information..."
    LATEST_TAG="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" |
        grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
    if [ -z "$LATEST_TAG" ]; then
        die "Could not resolve the latest GPY release from the GitHub API. Check your network, or pin a version: GPY_VERSION=v1.2.3 sh install-oneline.sh"
    fi
    VERSION="$LATEST_TAG"
    success "Latest version: $VERSION"
fi

# Fetch and check the shell files first, so a bad archive fails before any
# installed binary is replaced. The trap names files created further down too.
trap 'rm -f "${TEMP_BINARY:-}" "${TEMP_CLI_BINARY:-}" "${STAGED_AGENT:-}" "${STAGED_CLI:-}" 2>/dev/null; rm -rf "${TEMP_DIR:-}" 2>/dev/null' EXIT

info "Downloading $CURRENT_SHELL integration files..."

# Shell files come only from the release archive, verified before extraction.
SHELL_ARCHIVE_NAME="gpy-shell-files.tar.gz"
SHELL_ARCHIVE_URL="https://github.com/$REPO/releases/download/$VERSION/$SHELL_ARCHIVE_NAME"

TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-install.XXXXXX") || die "Failed to create a temp directory for downloads"

command -v tar >/dev/null 2>&1 || die "tar not found; it is required to unpack the shell files."

STAGE_DIR="$TEMP_DIR/share"
STAGE_SHELL_DIR="$STAGE_DIR/$CURRENT_SHELL"
TEMP_SHELL_ARCHIVE="$TEMP_DIR/$SHELL_ARCHIVE_NAME"
mkdir -p "$STAGE_DIR"

fetch_to "$SHELL_ARCHIVE_URL" "$TEMP_SHELL_ARCHIVE" ||
    die "Failed to download $SHELL_ARCHIVE_NAME from the $VERSION release. Check your internet connection, or see https://github.com/$REPO/releases"
verify_download "$TEMP_SHELL_ARCHIVE" "$SHELL_ARCHIVE_URL" "$SHELL_ARCHIVE_NAME"

tar -xzf "$TEMP_SHELL_ARCHIVE" -C "$STAGE_DIR" ||
    die "Failed to unpack $SHELL_ARCHIVE_NAME"

# Check required files before anything is installed.
case "$CURRENT_SHELL" in
    fish) REQUIRED_SHELL_FILES="conf.d/gpy_init.fish functions/fish_prompt.fish core/init.fish" ;;
    zsh) REQUIRED_SHELL_FILES="gpy.zsh core/init.zsh" ;;
    bash) REQUIRED_SHELL_FILES="gpy.bash core/init.bash" ;;
esac
for required_file in $REQUIRED_SHELL_FILES; do
    [ -f "$STAGE_SHELL_DIR/$required_file" ] ||
        die "$SHELL_ARCHIVE_NAME has no $CURRENT_SHELL/$required_file; refusing to install an incomplete integration."
done

# Keep only the backup just written. Compares names, not mtimes (same-second
# stamps tie).
prune_backups() {
    prune_dest="$1"
    prune_keep="$2"
    for prune_file in "$prune_dest".backup.*; do
        [ -e "$prune_file" ] || continue
        [ "$prune_file" = "$prune_keep" ] && continue
        rm -f "$prune_file"
    done
}

info "Installing GPY agent..."

mkdir -p "$INSTALL_DIR"

# mktemp keeps the name unpredictable to other local users; the EXIT trap
# removes partial downloads.
BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY_NAME"
TEMP_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-agent.XXXXXX") || die "Failed to create a temp file for the download"
STAGED_AGENT="$INSTALL_DIR/.gpy-agent.new.$$"
STAGED_CLI="$INSTALL_DIR/.gpy.new.$$"

info "Downloading agent from $BINARY_URL..."
if ! fetch_to "$BINARY_URL" "$TEMP_BINARY"; then
    die "Failed to download $BINARY_NAME from the $VERSION release. Check your internet connection, or see https://github.com/$REPO/releases"
fi

verify_download "$TEMP_BINARY" "$BINARY_URL" "$BINARY_NAME"

# Run the verified binary from a staging path before it replaces the install,
# so one that cannot run here (newer glibc, wrong libc) leaves the old install
# intact. Staging in $INSTALL_DIR avoids a noexec /tmp and keeps mv atomic.
chmod +x "$TEMP_BINARY"
mv "$TEMP_BINARY" "$STAGED_AGENT"
clear_quarantine "$STAGED_AGENT"

if ! "$STAGED_AGENT" --version >/dev/null 2>&1; then
    rm -f "$STAGED_AGENT"
    die "Agent binary installation failed (binary not functional); the existing install was left untouched"
fi

# Back up only once a working replacement exists.
if [ -f "$INSTALL_DIR/gpy-agent" ]; then
    BACKUP_PATH="$INSTALL_DIR/gpy-agent.backup.$(date +%Y%m%d_%H%M%S)"
    info "Backing up existing agent to $BACKUP_PATH"
    cp "$INSTALL_DIR/gpy-agent" "$BACKUP_PATH"
    prune_backups "$INSTALL_DIR/gpy-agent" "$BACKUP_PATH"
fi

# mv, not cp: a rename replaces the directory entry, so it is ETXTBSY-safe
# while the old agent is running.
mv "$STAGED_AGENT" "$INSTALL_DIR/gpy-agent"
clear_quarantine "$INSTALL_DIR/gpy-agent"

success "Agent binary installed to $INSTALL_DIR/gpy-agent"

info "Installing GPY CLI (gpy)..."

CLI_BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$CLI_BINARY_NAME"
TEMP_CLI_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-cli.XXXXXX") || die "Failed to create a temp file for the CLI download"

CLI_DOWNLOADED=0
info "Downloading CLI from $CLI_BINARY_URL..."
if fetch_to "$CLI_BINARY_URL" "$TEMP_CLI_BINARY"; then
    CLI_DOWNLOADED=1
fi

if [ "$CLI_DOWNLOADED" -eq 1 ]; then
    verify_download "$TEMP_CLI_BINARY" "$CLI_BINARY_URL" "$CLI_BINARY_NAME"

    # Stage and check before replacing the installed gpy.
    chmod +x "$TEMP_CLI_BINARY"
    mv "$TEMP_CLI_BINARY" "$STAGED_CLI"
    clear_quarantine "$STAGED_CLI"

    if "$STAGED_CLI" --version >/dev/null 2>&1; then
        if [ -f "$INSTALL_DIR/gpy" ]; then
            CLI_BACKUP_PATH="$INSTALL_DIR/gpy.backup.$(date +%Y%m%d_%H%M%S)"
            info "Backing up existing CLI to $CLI_BACKUP_PATH"
            cp "$INSTALL_DIR/gpy" "$CLI_BACKUP_PATH"
            prune_backups "$INSTALL_DIR/gpy" "$CLI_BACKUP_PATH"
        fi

        # mv, not cp: ETXTBSY-safe if an old gpy is mid-execution.
        mv "$STAGED_CLI" "$INSTALL_DIR/gpy"
        clear_quarantine "$INSTALL_DIR/gpy"
        success "CLI binary installed to $INSTALL_DIR/gpy"
    else
        rm -f "$STAGED_CLI"
        die "Downloaded CLI binary is not functional on this host; the existing gpy (if any) was left untouched"
    fi
else
    die "Could not download the gpy CLI ($CLI_BINARY_URL). It is required: it installs the shell integration."
fi

case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *":$INSTALL_DIR:"*) ;;
    *)
        warn "$INSTALL_DIR is not in your PATH"
        warn "Add this to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

# gpy setup reads confirmation from /dev/tty, so it can prompt despite the pipe.
info "Setting up $CURRENT_SHELL integration..."
"$INSTALL_DIR/gpy" setup --shell "$CURRENT_SHELL" --from "$STAGE_DIR" || die "gpy setup failed; the binaries are installed but $CURRENT_SHELL is not wired up. After fixing the problem above, run: $INSTALL_DIR/gpy setup --shell $CURRENT_SHELL --from <checkout of $REPO at $VERSION>"

# Always run `start`: it is the only path that restarts a version-mismatched daemon.
info "Starting GPY agent..."

if "$INSTALL_DIR/gpy-agent" start >/dev/null 2>&1; then
    success "GPY agent started"
else
    warn "Could not start agent automatically (will start on next shell session)"
fi

rm -rf "$TEMP_DIR"

echo ""
success "GPY installed successfully"
echo ""

info "Installation summary:"
echo "   • Agent: $INSTALL_DIR/gpy-agent"
if [ -x "$INSTALL_DIR/gpy" ]; then
    echo "   • CLI:   $INSTALL_DIR/gpy"
fi
echo "   • Shell files: $SHELL_CONFIG_DIR"
echo ""

info "Configuration and help:"
echo "   • View config: cat $CONFIG_HOME/gpy/config.toml"
echo "   • Agent commands: gpy-agent --help"
echo "   • Documentation: https://github.com/jpease/gpy"
echo "   • Check agent status: gpy-agent status"
echo "   • View logs: tail -f $CACHE_HOME/gpy/agent.log"
# Download-then-run, not `curl | sh`: the uninstaller reads confirmation from stdin.
UNINSTALL_URL="https://github.com/$REPO/releases/download/$VERSION"
echo "   • Uninstall: curl -fsSL $UNINSTALL_URL/uninstall.sh -o /tmp/gpy-uninstall.sh && GPY_SHELL=$CURRENT_SHELL sh /tmp/gpy-uninstall.sh"
echo ""
}

main "$@"
