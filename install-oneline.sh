#!/bin/sh
# GPY one-line installer, published as a release asset:
#   curl -fsSL https://github.com/jpease/gpy/releases/latest/download/install-oneline.sh | sh
# package-release.sh stamps RELEASE_VERSION; empty (source checkout) means the
# latest release. GPY_VERSION overrides either.

set -e

RELEASE_VERSION=""

info() { printf '%s\n' "$1"; }
ok() { printf 'ok: %s\n' "$1"; }
warn() { printf 'warn: %s\n' "$1" >&2; }
error() { printf 'error: %s\n' "$1" >&2; }

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
        curl) curl -fsSL "$1" -o "$2" ;;
        wget) wget -q "$1" -O "$2" ;;
        *) return 1 ;;
    esac
}

fetch_stdout() {
    case "$DOWNLOADER" in
        curl) curl -fsSL "$1" ;;
        wget) wget -qO- "$1" ;;
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

# A missing or empty sidecar aborts like a mismatch. The sidecar comes from the
# same release, so this catches corruption, not a compromised release; verify
# that with gh attestation verify (docs/INSTALL.md).
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

    ok "Verified $verify_label checksum"
}

# Gatekeeper SIGKILLs quarantined unsigned binaries. Call only after
# verify_download; scoped so other xattrs survive.
clear_quarantine() {
    if [ "$PLATFORM" = "darwin" ] && command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$1" 2>/dev/null || true
    fi
}

# Usage: prune_backups <dest> <keep>
# Keeps only the backup just written. Compares names, not mtimes (same-second
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

# Usage: stage_binary <asset> <dest-name> <label>
# Downloads and verifies the asset and runs it from a staging path, so one that
# cannot run here (newer glibc, wrong libc) is found before any installed binary
# is replaced. Staging in $INSTALL_DIR avoids a noexec /tmp and keeps mv atomic.
# Records the staged path in STAGED_AGENT or STAGED_CLI.
stage_binary() {
    sb_asset="$1"
    sb_label="$3"
    sb_url="https://github.com/$REPO/releases/download/$VERSION/$sb_asset"
    sb_download="$TEMP_DIR/$2.download"
    sb_staged="$INSTALL_DIR/.$2.new.$$"

    case "${2##*/}" in
        gpy-agent) STAGED_AGENT=$sb_staged ;;
        gpy) STAGED_CLI=$sb_staged ;;
    esac

    info "Downloading $sb_label from $sb_url..."
    fetch_to "$sb_url" "$sb_download" ||
        die "Failed to download $sb_url. Check your internet connection, or see https://github.com/$REPO/releases"
    verify_download "$sb_download" "$sb_url" "$sb_asset"

    chmod +x "$sb_download"
    mv "$sb_download" "$sb_staged"
    clear_quarantine "$sb_staged"

    if ! "$sb_staged" --version >/dev/null 2>&1; then
        die "The $sb_label binary is not functional on this host; the existing install was left untouched"
    fi
}

# Usage: commit_binary <staged> <dest-name> <label>
commit_binary() {
    cb_staged="$1"
    cb_dest="$INSTALL_DIR/$2"
    cb_label="$3"

    # Back up only once a working replacement exists.
    if [ -f "$cb_dest" ]; then
        cb_backup="$cb_dest.backup.$(date +%Y%m%d_%H%M%S)"
        info "Backing up existing $cb_label to $cb_backup"
        cp "$cb_dest" "$cb_backup"
        prune_backups "$cb_dest" "$cb_backup"
    fi

    # mv, not cp: a rename replaces the directory entry, so it is ETXTBSY-safe
    # while the old binary is running.
    mv "$cb_staged" "$cb_dest"
    ok "$cb_label installed to $cb_dest"
}

usage() {
    cat <<'EOF'
Usage: sh install-oneline.sh [-h|--help]

Takes no other options. Configure with environment variables:
  GPY_SHELL=fish|zsh|bash  Shell to wire up (default: basename of $SHELL).
                           Any other shell installs the binaries only.
  GPY_VERSION=v1.2.3       Release to install (default: latest).
  GPY_NERD_FONT=1|0|unknown  Set the icon style without asking.

Piped: curl -fsSL <url>/install-oneline.sh | GPY_SHELL=zsh sh
EOF
}

main() {
    for arg in "$@"; do
        case "$arg" in
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "unknown argument: $arg"
                usage >&2
                exit 2
                ;;
        esac
    done

    info "GPY one-line installer"
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
            echo "Build from source: https://github.com/jpease/gpy/blob/main/docs/INSTALL.md#building-from-source" >&2
            exit 1
            ;;
    esac

    ok "Platform: $PLATFORM ($ARCH)"

    CURRENT_SHELL="${GPY_SHELL:-${SHELL##*/}}"

    if [ -n "$GPY_SHELL" ]; then
        info "Using shell from GPY_SHELL environment variable: $CURRENT_SHELL"
    fi

    case "$CURRENT_SHELL" in
        fish|zsh|bash)
            ok "Detected shell: $CURRENT_SHELL"
            ;;
        *)
            warn "Unsupported shell '$CURRENT_SHELL'${GPY_SHELL:+ (from GPY_SHELL)}: installing the binaries only. For shell integration re-run with GPY_SHELL=fish, zsh or bash"
            CURRENT_SHELL=""
            ;;
    esac

    INSTALL_DIR="$HOME/.local/bin"

    # Fish honors XDG_CONFIG_HOME for its own config dir, so the install must too.
    CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
    CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"

    if [ -n "$CURRENT_SHELL" ]; then
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
    fi

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
        ok "Latest version: $VERSION"
    fi

    # Stage and test both binaries (and fetch the shell files) before replacing
    # anything, so a bad archive fails before any installed binary is replaced.
    trap 'rm -f "${STAGED_AGENT:-}" "${STAGED_CLI:-}" 2>/dev/null; rm -rf "${TEMP_DIR:-}" 2>/dev/null' EXIT

    TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-install.XXXXXX") || die "Failed to create a temp directory for downloads"

    if [ -n "$CURRENT_SHELL" ]; then
        info "Downloading $CURRENT_SHELL integration files..."

        # Shell files come only from the release archive, verified before extraction.
        SHELL_ARCHIVE_NAME="gpy-shell-files.tar.gz"
        SHELL_ARCHIVE_URL="https://github.com/$REPO/releases/download/$VERSION/$SHELL_ARCHIVE_NAME"

        command -v tar >/dev/null 2>&1 || die "tar not found; it is required to unpack the shell files."

        STAGE_DIR="$TEMP_DIR/share"
        STAGE_SHELL_DIR="$STAGE_DIR/$CURRENT_SHELL"
        TEMP_SHELL_ARCHIVE="$TEMP_DIR/$SHELL_ARCHIVE_NAME"
        mkdir -p "$STAGE_DIR"

        fetch_to "$SHELL_ARCHIVE_URL" "$TEMP_SHELL_ARCHIVE" ||
            die "Failed to download $SHELL_ARCHIVE_URL. Check your internet connection, or see https://github.com/$REPO/releases"
        verify_download "$TEMP_SHELL_ARCHIVE" "$SHELL_ARCHIVE_URL" "$SHELL_ARCHIVE_NAME"

        tar -xzf "$TEMP_SHELL_ARCHIVE" -C "$STAGE_DIR" ||
            die "Failed to unpack $SHELL_ARCHIVE_NAME"

        # Check required files before anything is installed.
        case "$CURRENT_SHELL" in
            fish) REQUIRED_SHELL_FILES="conf.d/gpy_init.fish functions/fish_prompt.fish core/init.fish core/flags.fish" ;;
            zsh) REQUIRED_SHELL_FILES="gpy.zsh core/init.zsh" ;;
            bash) REQUIRED_SHELL_FILES="gpy.bash core/init.bash" ;;
        esac
        for required_file in $REQUIRED_SHELL_FILES; do
            [ -f "$STAGE_SHELL_DIR/$required_file" ] ||
                die "$SHELL_ARCHIVE_NAME has no $CURRENT_SHELL/$required_file; refusing to install an incomplete integration."
        done
    fi

    mkdir -p "$INSTALL_DIR"

    # Stage both binaries before committing either: a bad CLI must not leave a
    # new agent beside the old CLI.
    info "Staging GPY agent..."
    stage_binary "$BINARY_NAME" gpy-agent "Agent binary"

    info "Staging GPY CLI (gpy)..."
    stage_binary "$CLI_BINARY_NAME" gpy "CLI binary"

    # The agent goes first: the CLI's setup step starts it.
    commit_binary "$STAGED_AGENT" gpy-agent "Agent binary"
    commit_binary "$STAGED_CLI" gpy "CLI binary"

    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            warn "Add this to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
            ;;
    esac

    # gpy setup reads confirmation from /dev/tty, so it can prompt despite the pipe.
    if [ -n "$CURRENT_SHELL" ]; then
        info "Setting up $CURRENT_SHELL integration..."
        "$INSTALL_DIR/gpy" setup --shell "$CURRENT_SHELL" --from "$STAGE_DIR" || die "gpy setup failed; the binaries are installed but $CURRENT_SHELL is not wired up. After fixing the problem above, run: $INSTALL_DIR/gpy setup --shell $CURRENT_SHELL --from <checkout of $REPO at $VERSION>"
    fi

    # Always run `start`: it is the only path that restarts a version-mismatched daemon.
    info "Starting GPY agent..."

    rc=0
    "$INSTALL_DIR/gpy-agent" start >/dev/null 2>&1 || rc=$?
    case $rc in
        0) ok "GPY agent started" ;;
        6) info "agent disabled in config (agent.enabled = false); not started" ;;
        *) warn "Could not start agent automatically (will start on next shell session)" ;;
    esac

    echo ""
    if [ -n "$CURRENT_SHELL" ]; then
        ok "GPY installed successfully"
    else
        warn "GPY binaries installed; shell integration was NOT set up. Re-run with GPY_SHELL=fish, zsh or bash"
    fi
    echo ""
    echo "Agent:         $INSTALL_DIR/gpy-agent"
    echo "CLI:           $INSTALL_DIR/gpy"
    if [ -n "$CURRENT_SHELL" ]; then
        echo "Shell files:   $SHELL_CONFIG_DIR"
    fi
    echo "Config:        $CONFIG_HOME/gpy/config.toml"
    echo "Agent status:  gpy-agent status"
    echo "Agent logs:    tail -f $CACHE_HOME/gpy/agent.log"
    echo "Documentation: https://github.com/jpease/gpy"
    # Download-then-run, not `curl | sh`: the uninstaller reads confirmation from stdin.
    UNINSTALL_URL="https://github.com/$REPO/releases/download/$VERSION"
    echo "Uninstall:     curl -fsSL $UNINSTALL_URL/uninstall.sh -o /tmp/gpy-uninstall.sh && GPY_SHELL=$CURRENT_SHELL sh /tmp/gpy-uninstall.sh"
}

main "$@"
