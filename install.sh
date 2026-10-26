#!/bin/bash
# Install GPY from an extracted release archive: run `bash install.sh` inside it.
set -eE

info() { printf '%s\n' "$1"; }
ok() { printf 'ok: %s\n' "$1"; }
warn() { printf 'warn: %s\n' "$1" >&2; }
error() { printf 'error: %s\n' "$1" >&2; }

die() {
    error "$1"
    exit 1
}

# Names the step in progress, so a failure points at what actually broke.
CURRENT_STEP="starting"

handle_error() {
    local exit_code=$1
    echo "" >&2
    error "installation failed during: $CURRENT_STEP (exit $exit_code)"
    echo "Run with bash -x install.sh for detailed output." >&2
    echo "Other install methods:" >&2
    echo "  One-line installer: curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh" >&2
    echo "  Homebrew:           brew install jpease/tap/gpy, then gpy setup" >&2
    echo "  Source:             docs/INSTALL.md#building-from-source" >&2
    exit "$exit_code"
}

trap 'handle_error $?' ERR

# Without a SHA-256 tool nothing can be verified, so this is fatal.
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    die "no SHA-256 tool found (need sha256sum or shasum); GPY verifies every binary before installing it."
fi

# Fails closed: a missing sidecar is treated like a mismatch, since "never
# published" and "stripped" look identical here. This only proves the archive
# is self-consistent; verify the archive itself against SHA256SUMS or with
# `gh attestation verify` (docs/INSTALL.md).
verify_checksum() {
    file_path="$1"
    label="$2"
    sidecar="$file_path.sha256"

    if [ ! -f "$sidecar" ]; then
        die "no checksum for $label binary ($sidecar not found). This package is incomplete or was repacked; re-download the release archive from https://github.com/jpease/gpy/releases"
    fi

    expected=$(awk '{print $1}' "$sidecar")
    actual=$(sha256_of "$file_path")

    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        die "checksum mismatch for $label binary (expected $expected, actual $actual); refusing to install a binary that does not match its published checksum."
    fi

    ok "Verified $label binary checksum"
}

# Usage: install_binary <source_path> <dest_path> <label>
# Runs the verified binary from a staging path before it replaces the install,
# so one that cannot run here leaves the old install intact. Staging beside the
# destination keeps mv atomic.
install_binary() {
    src_path="$1"
    dest_path="$2"
    label="$3"

    CURRENT_STEP="installing the $label binary"

    if [ ! -f "$src_path" ]; then
        die "binary $src_path not found. Get a release archive from https://github.com/jpease/gpy/releases or build from source: https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
    fi

    # Verify first: a failed check must leave the machine untouched.
    verify_checksum "$src_path" "$label"

    staged="$(dirname "$dest_path")/.$(basename "$dest_path").new.$$"
    trap 'rm -f "$staged"' EXIT

    if command -v install >/dev/null 2>&1; then
        install -m 755 "$src_path" "$staged"
    else
        cp "$src_path" "$staged"
        chmod +x "$staged"
    fi

    # Gatekeeper SIGKILLs quarantined unsigned binaries. Runs only after
    # verify_checksum; scoped so other xattrs survive.
    if command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$staged" 2>/dev/null || true
    fi

    if ! "$staged" --version >/dev/null 2>&1; then
        die "the $label binary is not functional on this host; the existing install was left untouched"
    fi

    # Back up only once a working replacement exists; keep just the newest.
    if [ -f "$dest_path" ]; then
        new_backup="$dest_path.backup.$(date +%Y%m%d_%H%M%S)"
        info "Backing up existing $label binary to $new_backup"
        cp "$dest_path" "$new_backup"
        for old_backup in "$dest_path".backup.*; do
            [ -e "$old_backup" ] || continue
            [ "$old_backup" = "$new_backup" ] && continue
            rm -f "$old_backup"
        done
    fi

    # mv, not cp: a rename replaces the directory entry, so it is ETXTBSY-safe
    # while the old binary is running.
    mv "$staged" "$dest_path"
    ok "$label binary installed to $dest_path"
}

info "GPY installer"
echo ""

PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# The rc-file block sources this path inside double quotes; refuse characters
# that would break out rather than escape them.
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
case "$CONFIG_HOME" in
    *[\"\$\`\\]* | *"
"*)
        die "cannot install: $CONFIG_HOME contains a quote, \$, backtick, backslash or newline, which cannot be sourced safely from a shell rc file. Use an XDG_CONFIG_HOME without those characters."
        ;;
esac

# Unlike install-oneline.sh, this also resolves Windows assets.
case $PLATFORM in
    linux)
        case $ARCH in
            x86_64) BINARY="gpy-agent-linux-x86_64"; CLI_BINARY="gpy-linux-x86_64" ;;
            aarch64|arm64) BINARY="gpy-agent-linux-aarch64"; CLI_BINARY="gpy-linux-aarch64" ;;
            *) die "unsupported architecture: $ARCH" ;;
        esac
        ;;
    darwin)
        case $ARCH in
            x86_64) BINARY="gpy-agent-macos-x86_64"; CLI_BINARY="gpy-macos-x86_64" ;;
            arm64) BINARY="gpy-agent-macos-aarch64"; CLI_BINARY="gpy-macos-aarch64" ;;
            *) die "unsupported architecture: $ARCH" ;;
        esac
        ;;
    mingw*|msys*|cygwin*)
        BINARY="gpy-agent-windows-x86_64.exe"
        CLI_BINARY="gpy-windows-x86_64.exe"
        ;;
    *)
        die "unsupported platform: $PLATFORM. Build from source: https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
        ;;
esac

info "Installing GPY for $PLATFORM ($ARCH)"
info "Agent binary: $BINARY"
info "CLI binary:   $CLI_BINARY"
echo ""

INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"
install_binary "bin/$BINARY" "$INSTALL_DIR/gpy-agent" "agent"
install_binary "bin/$CLI_BINARY" "$INSTALL_DIR/gpy" "CLI"
trap - EXIT

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    warn "$INSTALL_DIR is not in your PATH"
    warn "Add this to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# Shell is detected from $SHELL; GPY_SHELL overrides.
CURRENT_STEP="gpy setup (shell integration; binaries are already installed)"
info "Installing shell integration..."
"$INSTALL_DIR/gpy" setup --from "$(pwd)"

# Restart so an old daemon picks up the new binary; `start` is a no-op when the
# running version already matches.
CURRENT_STEP="restarting the agent"
info "Restarting GPY agent..."
if "$INSTALL_DIR/gpy-agent" start >/dev/null 2>&1; then
    ok "GPY agent started"
else
    warn "could not start the agent automatically (it will start on the next shell session)"
fi

echo ""
ok "GPY installed successfully"
echo ""
echo "Agent:     $INSTALL_DIR/gpy-agent"
echo "CLI:       $INSTALL_DIR/gpy"
echo "Verify:    gpy --version; gpy-agent --help"
echo "Uninstall: sh scripts/uninstall.sh (from this package; see docs/INSTALL.md)"
