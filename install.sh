#!/bin/bash
set -e

# Trap errors and provide recovery guidance
trap 'handle_error $? $LINENO' ERR

handle_error() {
    local exit_code=$1
    local line_no=$2
    echo ""
    echo "Installation failed at line $line_no with exit code $exit_code"
    echo ""
    echo "Troubleshooting steps:"
    echo "   1. Check if you have write permissions to ~/.local/bin"
    echo "   2. Verify Fish shell is installed: fish --version"
    echo "   3. Check if ~/.config/fish directory exists"
    echo "   4. Run with bash -x install.sh for detailed output"
    echo ""
    # Only channels that deliver the agent binary are listed. The Fisher
    # plugin ships shell files only, so it cannot fix a failure to place a
    # binary.
    echo "Alternative installation methods:"
    echo "   • One-line installer: curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | sh"
    echo "   • Homebrew (Fish): brew install jpease/tap/gpy"
    echo "   • Manual: Use install-dev.fish if you have the source"
    echo ""
    exit "$exit_code"
}

# Resolve the SHA-256 tool once. Without sha256sum or shasum nothing can be
# verified, and an unverified binary is never installed, so this is fatal.
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    echo "Error: no SHA-256 tool found (need sha256sum or shasum)"
    echo "   GPY verifies every binary before installing it and cannot continue."
    exit 1
fi

# Verify a packaged binary against the sidecar scripts/package-release.sh wrote
# next to it. Fails closed: a missing sidecar is treated exactly like a
# mismatched one, because "the release forgot to publish verification metadata"
# and "someone stripped it" are indistinguishable from here.
#
# This checks the archive against itself, so it catches a corrupted download or
# an edited archive member. It cannot vouch for the archive as a whole -- for
# that, verify gpy-release.tar.gz against the release's SHA256SUMS, or check the
# build attestation with `gh attestation verify`. Both are documented in
# docs/INSTALL.md.
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

# Install a single binary from a local bin/ source to a destination on PATH:
# verify, back up any existing copy, unlink before writing (ETXTBSY-safe for a
# running daemon), prefer `install` for correct perms, clear the macOS
# quarantine flag, check `--version`, and restore the newest backup on failure.
#
# <required> decides what happens when the source binary is absent: the agent
# daemon is required (missing -> fatal), while the gpy CLI is optional
# (missing -> warn and skip; it only powers shell completions/subcommands),
# the same split install-oneline.sh makes for its downloads.
# Usage: install_binary <source_path> <dest_path> <label> <required:1|0>
install_binary() {
    src_path="$1"
    dest_path="$2"
    label="$3"
    required="$4"

    if [ ! -f "$src_path" ]; then
        if [ "$required" = "1" ]; then
            echo "Error: Binary $src_path not found"
            echo ""
            # Fisher ships no binary, so it is not offered (see handle_error).
            echo "Alternative installation methods:"
            echo "   • Release archive: https://github.com/jpease/gpy/releases"
            echo "   • Build from source: https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
            exit 1
        fi
        echo "Warning: $label binary ($src_path) not found in package; skipping (shell completions may be unavailable)"
        return 0
    fi

    # Verify before anything is backed up, unlinked, or written: a failed
    # verification must leave the machine exactly as it was.
    verify_checksum "$src_path" "$label"

    echo "Installing $label binary..."

    new_backup=""
    if [ -f "$dest_path" ]; then
        echo "Backing up existing $label binary..."
        new_backup="$dest_path.backup.$(date +%Y%m%d_%H%M%S)"
        cp "$dest_path" "$new_backup"
    fi

    # Unlink the destination before writing the new binary. A previous process
    # may still be running with the old binary mapped; overwriting the existing
    # inode in place (the default for both `install` and `cp`) can fail with
    # ETXTBSY on Linux while it's executing. Removing the directory entry first
    # means the old process keeps its already-open inode while the new binary
    # gets a fresh one at the same path.
    rm -f "$dest_path"

    # Use install command if available (handles permissions correctly)
    if command -v install >/dev/null 2>&1; then
        install -m 755 "$src_path" "$dest_path"
    else
        cp "$src_path" "$dest_path"
        chmod +x "$dest_path"
    fi

    # Drop the macOS quarantine flag, which otherwise makes Gatekeeper SIGKILL
    # an unsigned binary extracted from a downloaded archive. This runs only
    # after verify_checksum has matched the binary against its published
    # digest, so it never lowers a barrier on an unverified file. Scoped to
    # com.apple.quarantine so other extended attributes (provenance metadata)
    # survive.
    if command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$dest_path" 2>/dev/null || true
    fi

    # Verify binary works
    if "$dest_path" --version >/dev/null 2>&1; then
        echo "$label binary installed and functional"
        # Keep only the backup just written; older ones are never used. Only
        # pruned on success so the restore branch below can still use them.
        if [ -n "$new_backup" ]; then
            for old_backup in "$dest_path".backup.*; do
                [ -e "$old_backup" ] || continue
                [ "$old_backup" = "$new_backup" ] && continue
                rm -f "$old_backup"
            done
        fi
    else
        echo "Error: $label binary installation failed or not functional"
        # Pick the newest backup explicitly: a `[ -f pattern* ]` test breaks
        # once the glob expands to two or more names.
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

# Detect platform
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Fish honors XDG_CONFIG_HOME for its own config dir, so the install must too.
FISH_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/fish"

# The config.fish block sources this path inside double quotes; `"`, `$`, a
# backtick, a backslash or a newline would break out of them. Refuse before
# anything is written rather than escape.
case "$FISH_CONFIG_DIR" in
    *[\"\$\`\\]* | *"
"*)
        echo "Error: Cannot install: $FISH_CONFIG_DIR contains a quote, \$, backtick, backslash or newline,"
        echo "   which cannot be sourced safely from config.fish. Use an XDG_CONFIG_HOME without those characters."
        exit 1
        ;;
esac

# Each platform/arch resolves BOTH the agent daemon binary ($BINARY) and the
# user-facing CLI binary ($CLI_BINARY), shipped as a parallel pair of assets
# and installed side by side; the CLI powers shell completions and `gpy`
# subcommands. Unlike install-oneline.sh, this table resolves Windows names
# too (the mingw*/msys*/cygwin* arm).
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
        # No release binary exists for this platform and the Fisher plugin
        # ships none, so building from source is the only path to an agent.
        echo "Build from source:"
        echo "   https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source"
        exit 1
        ;;
esac

echo "Installing GPY for $PLATFORM ($ARCH)..."
echo "Using agent binary: $BINARY"
echo "Using CLI binary:   $CLI_BINARY"
echo ""

# The agent daemon is required; the CLI (`gpy`) is optional.
mkdir -p ~/.local/bin
install_binary "bin/$BINARY" ~/.local/bin/gpy-agent "agent" 1
install_binary "bin/$CLI_BINARY" ~/.local/bin/gpy "CLI" 0

# Check if ~/.local/bin is in PATH
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo "Warning: ~/.local/bin is not in your PATH"
    echo "   Add this to your shell profile:"
    echo "   export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# Install Fish files (with error handling)
echo "Installing Fish integration..."
mkdir -p "$FISH_CONFIG_DIR/gpy"

# Try multiple potential source locations. Only the known Fish implementation
# subdirectories are copied (not "$source_dir"/*): the "." fallback matches
# whenever core/init.fish happens to sit at the invocation cwd, and a
# wildcard copy from there would drag the whole distribution tree (.git,
# gpy-agent/, bin/, ...) into the Fish config dir.
FISH_FILES_COPIED=0
for source_dir in "fish" "." "src"; do
    if [ -d "$source_dir" ] && [ -f "$source_dir/core/init.fish" ] 2>/dev/null; then
        echo "Found Fish files in $source_dir/"
        for sub in core segments functions conf.d; do
            if [ -d "$source_dir/$sub" ]; then
                cp -r "$source_dir/$sub" "$FISH_CONFIG_DIR/gpy/" 2>/dev/null
            fi
        done
        FISH_FILES_COPIED=1
        break
    fi
done

if [ $FISH_FILES_COPIED -eq 0 ]; then
    echo "Error: Fish files not found in expected locations"
    echo "   Searched: fish/, ., src/"
    echo "   You may need to:"
    echo "   1. Download Fish scripts separately"
    echo "   2. Use install-dev.fish instead if you have source code"
    exit 1
fi

# Fish autoloads `completions/<command>.fish` only for the command of that
# name, so the structural completions (regenerated from the installed CLI so
# they cannot drift from it) and the checked-in gpy-dynamic.fish glue (live
# theme/palette/segment names via `gpy __complete <kind>`) share ONE file,
# completions/gpy.fish. Without the CLI, gpy.fish is the glue alone.
echo "Installing Fish completions..."
mkdir -p "$FISH_CONFIG_DIR/completions"
# The glue is never autoloaded as its own file; remove any stray copy.
rm -f "$FISH_CONFIG_DIR/completions/gpy-dynamic.fish"
fish_completions_generated=0
if [ -x ~/.local/bin/gpy ]; then
    if ~/.local/bin/gpy completions fish >"$FISH_CONFIG_DIR/completions/gpy.fish" 2>/dev/null; then
        fish_completions_generated=1
        echo "Generated structural completions: $FISH_CONFIG_DIR/completions/gpy.fish"
    else
        echo "Warning: Could not generate gpy completions (gpy completions fish failed)"
    fi
else
    echo "Warning: gpy CLI not installed; skipping structural shell completions"
fi

if [ -f "$source_dir/completions/gpy-dynamic.fish" ]; then
    if [ "$fish_completions_generated" -eq 1 ]; then
        cat "$source_dir/completions/gpy-dynamic.fish" >>"$FISH_CONFIG_DIR/completions/gpy.fish"
    else
        cat "$source_dir/completions/gpy-dynamic.fish" >"$FISH_CONFIG_DIR/completions/gpy.fish"
    fi
    echo "Installed dynamic completions glue into: $FISH_CONFIG_DIR/completions/gpy.fish"
else
    echo "Warning: fish/completions/gpy-dynamic.fish not found in package; dynamic value completions unavailable"
fi

# Add to Fish config (idempotent)
FISH_CONFIG="$FISH_CONFIG_DIR/config.fish"
if [ ! -f "$FISH_CONFIG" ]; then
    mkdir -p "$FISH_CONFIG_DIR"
    touch "$FISH_CONFIG"
fi

if ! grep -qF "# >>> gpy-init >>>" "$FISH_CONFIG" 2>/dev/null; then
    echo "Adding GPY initialization to Fish config..."
    cat >> "$FISH_CONFIG" << EOF

# >>> gpy-init >>>
# GPY Prompt Enhancement
if status is-interactive
    source "$FISH_CONFIG_DIR/gpy/conf.d/gpy_init.fish"
end
# <<< gpy-init <<<
EOF
    echo "GPY initialization added to Fish config"
else
    echo "GPY already configured in Fish config"
fi

# Ensure Fish prompt function points to GPY implementation
FISH_FUNCTIONS_DIR="$FISH_CONFIG_DIR/functions"
FISH_PROMPT_TARGET="$FISH_FUNCTIONS_DIR/fish_prompt.fish"
FISH_PROMPT_SOURCE="$FISH_CONFIG_DIR/gpy/functions/fish_prompt.fish"

mkdir -p "$FISH_FUNCTIONS_DIR"

# Anything that is not already GPY's own link (regular file, or a foreign
# symlink such as a stow/yadm-managed prompt) is moved aside; `mv` keeps a
# symlink a symlink so uninstall can restore it.
if { [ -e "$FISH_PROMPT_TARGET" ] || [ -L "$FISH_PROMPT_TARGET" ]; } \
    && { [ ! -L "$FISH_PROMPT_TARGET" ] || [ "$(readlink "$FISH_PROMPT_TARGET")" != "$FISH_PROMPT_SOURCE" ]; }; then
    echo "Backing up existing fish_prompt implementation..."
    mv "$FISH_PROMPT_TARGET" "$FISH_PROMPT_TARGET.gpy-backup.$(date +%Y%m%d_%H%M%S)"
fi

if [ -L "$FISH_PROMPT_TARGET" ] && [ "$(readlink "$FISH_PROMPT_TARGET")" = "$FISH_PROMPT_SOURCE" ]; then
    echo "GPY prompt symlink already in place"
else
    echo "Linking GPY prompt function..."
    ln -sf "$FISH_PROMPT_SOURCE" "$FISH_PROMPT_TARGET"
fi

# Choose an icon style that renders on this machine before the first prompt.
# `gpy-agent init` detects whether a Nerd Font is available and, on a fresh
# install only (no existing config), writes ~/.config/gpy/config.toml with a
# matching `show_icons` — so a machine without a Nerd Font never renders tofu
# boxes on its very first prompt. It leaves any existing config
# untouched, and when a terminal is attached it asks the user to confirm the
# glyphs against a rendered sample. Non-fatal: a failure here just leaves the
# built-in default, so `|| ...` keeps the ERR trap from aborting the install.
echo "Configuring prompt icons..."
if ~/.local/bin/gpy-agent init; then
    :
else
    echo "Warning: Icon setup skipped (gpy-agent init failed); using built-in defaults"
fi

# Restart the agent so a previously running (old-version) daemon picks up
# the new binary/protocol. `gpy-agent start` is idempotent: a no-op if a
# matching version is already running, and restarts on a version mismatch.
echo "Restarting GPY agent..."
if ~/.local/bin/gpy-agent start >/dev/null 2>&1; then
    echo "GPY agent started"
else
    echo "Warning: Could not start agent automatically (will start on next shell session)"
fi

echo ""
echo "GPY installed successfully!"
echo ""
echo "Next steps:"
echo "   1. If you previously sourced a custom prompt, comment it out to see GPY"
echo "   2. Restart your Fish shell or run: exec fish"
echo "   3. Or reload config: source \"$FISH_CONFIG_DIR/config.fish\""
echo ""
echo "Configuration: $FISH_CONFIG_DIR/gpy/"
echo "Agent binary: ~/.local/bin/gpy-agent"
echo "CLI binary:   ~/.local/bin/gpy"
echo ""
echo "Test the installation:"
echo "   gpy --version"
echo "   gpy-agent --help"
echo ""
echo "Uninstall: fish scripts/uninstall.fish (from this package; see docs/INSTALL.md)"
