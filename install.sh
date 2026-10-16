#!/bin/bash
# Install GPY from an extracted release archive: run `bash install.sh` inside it.
set -e

trap 'handle_error $? $LINENO' ERR

handle_error() {
    local exit_code=$1
    local line_no=$2
    echo ""
    echo "Installation failed at line $line_no with exit code $exit_code"
    echo ""
    echo "Troubleshooting steps:"
    echo "   1. Check if you have write permissions to ~/.local/bin"
    echo "   2. Check that your config directory (\${XDG_CONFIG_HOME:-~/.config}) is writable"
    echo "   3. Run with bash -x install.sh for detailed output"
    echo ""
    # Fisher is omitted: it ships no binary.
    echo "Alternative installation methods:"
    echo "   • One-line installer: curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh"
    echo "   • Homebrew: brew install jpease/tap/gpy, then gpy setup"
    echo "   • Source: see docs/INSTALL.md#building-from-source"
    echo ""
    exit "$exit_code"
}

# Without a SHA-256 tool nothing can be verified, so this is fatal.
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    echo "Error: no SHA-256 tool found (need sha256sum or shasum)"
    echo "   GPY verifies every binary before installing it and cannot continue."
    exit 1
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
        echo "Error: no checksum for $label binary ($sidecar not found)"
        echo "   This package is incomplete or was repacked; refusing to install."
        echo "   Re-download the release archive from:"
        echo "   https://github.com/jpease/gpy/releases"
        exit 1
    fi

    expected=$(awk '{print $1}' "$sidecar")
    actual=$(sha256_of "$file_path")

    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        echo "Error: checksum mismatch for $label binary"
        echo "   expected: $expected"
        echo "   actual:   $actual"
        echo "   Refusing to install a binary that does not match its published checksum."
        exit 1
    fi

    echo "Verified $label binary checksum"
}

# Usage: install_binary <source_path> <dest_path> <label>
install_binary() {
    src_path="$1"
    dest_path="$2"
    label="$3"

    if [ ! -f "$src_path" ]; then
        echo "Error: Binary $src_path not found"
        echo ""
        echo "Alternative installation methods:"
        echo "   • Release archive: https://github.com/jpease/gpy/releases"
        echo "   • Build from source: https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
        exit 1
    fi

    # Verify first: a failed check must leave the machine untouched.
    verify_checksum "$src_path" "$label"

    echo "Installing $label binary..."

    new_backup=""
    if [ -f "$dest_path" ]; then
        echo "Backing up existing $label binary..."
        new_backup="$dest_path.backup.$(date +%Y%m%d_%H%M%S)"
        cp "$dest_path" "$new_backup"
    fi

    # Unlink first: overwriting a running binary's inode fails with ETXTBSY on
    # Linux; a fresh inode lets the old process keep running.
    rm -f "$dest_path"

    if command -v install >/dev/null 2>&1; then
        install -m 755 "$src_path" "$dest_path"
    else
        cp "$src_path" "$dest_path"
        chmod +x "$dest_path"
    fi

    # Gatekeeper SIGKILLs quarantined unsigned binaries. Runs only after
    # verify_checksum; scoped so other xattrs survive.
    if command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$dest_path" 2>/dev/null || true
    fi

    if "$dest_path" --version >/dev/null 2>&1; then
        echo "$label binary installed and functional"
        # Prune only on success; the failure path restores from these.
        if [ -n "$new_backup" ]; then
            for old_backup in "$dest_path".backup.*; do
                [ -e "$old_backup" ] || continue
                [ "$old_backup" = "$new_backup" ] && continue
                rm -f "$old_backup"
            done
        fi
    else
        echo "Error: $label binary installation failed or not functional"
        # `[ -f pattern* ]` breaks once the glob matches several names.
        # shellcheck disable=SC2012  # backups are timestamped names we generate; ls -t is the portable mtime sort
        LATEST_BACKUP=$(ls -t "$dest_path".backup.* 2>/dev/null | head -1)
        if [ -n "$LATEST_BACKUP" ]; then
            echo "Attempting to restore from backup..."
            cp "$LATEST_BACKUP" "$dest_path"
        fi
        exit 1
    fi
}

echo "GPY installer"
echo ""

PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# The rc-file block sources this path inside double quotes; refuse characters
# that would break out rather than escape them.
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
case "$CONFIG_HOME" in
    *[\"\$\`\\]* | *"
"*)
        echo "Error: Cannot install: $CONFIG_HOME contains a quote, \$, backtick, backslash or newline,"
        echo "   which cannot be sourced safely from a shell rc file. Use an XDG_CONFIG_HOME without those characters."
        exit 1
        ;;
esac

# Unlike install-oneline.sh, this also resolves Windows assets.
case $PLATFORM in
    linux)
        case $ARCH in
            x86_64) BINARY="gpy-agent-linux-x86_64"; CLI_BINARY="gpy-linux-x86_64" ;;
            aarch64|arm64) BINARY="gpy-agent-linux-aarch64"; CLI_BINARY="gpy-linux-aarch64" ;;
            *) echo "Error: Unsupported architecture: $ARCH"; exit 1 ;;
        esac
        ;;
    darwin)
        case $ARCH in
            x86_64) BINARY="gpy-agent-macos-x86_64"; CLI_BINARY="gpy-macos-x86_64" ;;
            arm64) BINARY="gpy-agent-macos-aarch64"; CLI_BINARY="gpy-macos-aarch64" ;;
            *) echo "Error: Unsupported architecture: $ARCH"; exit 1 ;;
        esac
        ;;
    mingw*|msys*|cygwin*)
        BINARY="gpy-agent-windows-x86_64.exe"
        CLI_BINARY="gpy-windows-x86_64.exe"
        ;;
    *)
        echo "Error: Unsupported platform: $PLATFORM"
        echo ""
        echo "Build from source:"
        echo "   https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
        exit 1
        ;;
esac

echo "Installing GPY for $PLATFORM ($ARCH)..."
echo "Using agent binary: $BINARY"
echo "Using CLI binary:   $CLI_BINARY"
echo ""

mkdir -p ~/.local/bin
install_binary "bin/$BINARY" ~/.local/bin/gpy-agent "agent"
install_binary "bin/$CLI_BINARY" ~/.local/bin/gpy "CLI"

if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo "Warning: ~/.local/bin is not in your PATH"
    echo "   Add this to your shell profile:"
    echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# Shell is detected from $SHELL; GPY_SHELL overrides.
echo "Installing shell integration..."
~/.local/bin/gpy setup --from "$(pwd)"

# Restart so an old daemon picks up the new binary; `start` is a no-op when the
# running version already matches.
echo "Restarting GPY agent..."
if ~/.local/bin/gpy-agent start >/dev/null 2>&1; then
    echo "GPY agent started"
else
    echo "Warning: Could not start agent automatically (will start on next shell session)"
fi

echo ""
echo "GPY installed successfully!"
echo ""
echo "Agent binary: ~/.local/bin/gpy-agent"
echo "CLI binary:   ~/.local/bin/gpy"
echo ""
echo "Test the installation:"
echo "   gpy --version"
echo "   gpy-agent --help"
echo ""
echo "Uninstall: sh scripts/uninstall.sh, or fish scripts/uninstall.fish (from this package; see docs/INSTALL.md)"
