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

# The downloader is resolved once; every fetch goes through fetch_to or
# fetch_stdout.
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
# Every branch fails closed: an attacker who can serve a binary can also serve
# no checksum, so an unreachable or empty sidecar aborts just like a mismatch.
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

# Drop the macOS quarantine flag so Gatekeeper does not SIGKILL an unsigned
# binary. Only ever called after verify_download has matched the file against
# its published digest, so it never lowers a barrier on an unverified download.
# Scoped to com.apple.quarantine so other extended attributes (provenance
# metadata) survive.
clear_quarantine() {
    if [ "$PLATFORM" = "darwin" ] && command -v xattr >/dev/null 2>&1; then
        xattr -d com.apple.quarantine "$1" 2>/dev/null || true
    fi
}

# Append a gpy-init marker block to rc file $1: header comment $2, body $3.
# Returns 1 without writing when the file already carries a block, so a re-run
# never stacks a second one.
append_gpy_block() {
    if grep -qF "# >>> gpy-init >>>" "$1" 2>/dev/null; then
        return 1
    fi
    printf '\n# >>> gpy-init >>>\n# %s\n%s\n# <<< gpy-init <<<\n' "$2" "$3" >>"$1"
}

# Write the CLI's structural completions (subcommand/flag names) for shell $1
# to $2, regenerated from the installed binary so they cannot drift from it.
# Best-effort: the prompt does not depend on the gpy CLI, so a missing CLI or a
# failed generation only warns and returns 1.
generate_completions() {
    if ! [ -x "$INSTALL_DIR/gpy" ]; then
        warn "gpy CLI not installed; skipping shell completions"
        return 1
    fi
    if "$INSTALL_DIR/gpy" completions "$1" >"$2" 2>/dev/null; then
        success "Generated structural completions: $2"
    else
        warn "Could not generate gpy completions (gpy completions $1 failed)"
        return 1
    fi
}

main() {
bold "GPY one-line installer"
echo ""

# Both requirements are fatal up front: there is no partial install worth
# starting if the machine cannot download or cannot verify what it downloads.
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
# user-facing CLI ($CLI_BINARY_NAME), shipped as a parallel pair of release
# assets; the CLI powers shell completions and `gpy` subcommands.
#
# No mingw*/msys*/cygwin* arm, unlike install.sh: this installer sets up
# Unix-socket shell integration, which has nothing to install on Windows.
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

# Fish honors XDG_CONFIG_HOME for its own config dir, so the install must too.
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
CACHE_HOME="${XDG_CACHE_HOME:-$HOME/.cache}"

# Fish uses $CONFIG_HOME/fish/gpy/ (same as install.sh); other shells use
# $CONFIG_HOME/gpy/{zsh,bash}/.
if [ "$CURRENT_SHELL" = "fish" ]; then
    SHELL_CONFIG_DIR="$CONFIG_HOME/fish/gpy"
else
    SHELL_CONFIG_DIR="$CONFIG_HOME/gpy/$CURRENT_SHELL"
fi

# The rc block sources this path inside double quotes, which fish, zsh and
# bash all read identically -- except that `"`, `$`, a backtick, a backslash
# or a newline still break out of them. Rather than escape for three shells,
# refuse such a path before any file is written.
case "$SHELL_CONFIG_DIR" in
    *[\"\$\`\\]* | *"
"*)
        die "Cannot install: $SHELL_CONFIG_DIR contains a quote, \$, backtick, backslash or newline, which cannot be sourced safely from a shell rc file. Use an XDG_CONFIG_HOME without those characters."
        ;;
esac

# GitHub release information
REPO="jpease/gpy"
VERSION="${GPY_VERSION:-latest}"

# Resolve the release tag or stop: never fall back to a mutable branch, which
# would pair an unpinned, unverifiable binary with unreleased shell files.
if [ "$VERSION" = "latest" ]; then
    info "Fetching latest release information..."
    LATEST_TAG="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" |
        grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
    if [ -z "$LATEST_TAG" ]; then
        die "Could not resolve the latest GPY release from the GitHub API. Check your network, or pin a version: curl -sS https://raw.githubusercontent.com/jpease/gpy/main/install-oneline.sh | GPY_VERSION=v1.2.3 sh"
    fi
    VERSION="$LATEST_TAG"
    success "Latest version: $VERSION"
fi

# Delete every "$1".backup.* except the one just written ($2): only the most
# recent previous version is worth keeping, and each backup is a full binary.
# Compares names rather than mtimes (same-second stamps tie).
prune_backups() {
    prune_dest="$1"
    prune_keep="$2"
    for prune_file in "$prune_dest".backup.*; do
        [ -e "$prune_file" ] || continue
        [ "$prune_file" = "$prune_keep" ] && continue
        rm -f "$prune_file"
    done
}

# Install agent binary
info "Installing GPY agent..."

mkdir -p "$INSTALL_DIR"

# mktemp (not a fixed /tmp path) so the name is unpredictable to other local
# users; the EXIT trap removes partial downloads when the install dies.
BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY_NAME"
TEMP_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-agent.XXXXXX") || die "Failed to create a temp file for the download"
STAGED_AGENT="$INSTALL_DIR/.gpy-agent.new.$$"
STAGED_CLI="$INSTALL_DIR/.gpy.new.$$"
trap 'rm -f "$TEMP_BINARY" "$STAGED_AGENT" 2>/dev/null' EXIT

info "Downloading agent from $BINARY_URL..."
if ! fetch_to "$BINARY_URL" "$TEMP_BINARY"; then
    die "Failed to download $BINARY_NAME from the $VERSION release. Check your internet connection, or see https://github.com/$REPO/releases"
fi

verify_download "$TEMP_BINARY" "$BINARY_URL" "$BINARY_NAME"

# Stage the verified download beside its destination and run it there BEFORE
# it replaces the installed agent. Staging in $INSTALL_DIR rather than
# $TMPDIR avoids a noexec /tmp and keeps the final mv an atomic same-filesystem
# rename. A binary that cannot run on this host (newer glibc, wrong libc or
# userland) is dropped and the working install is left exactly as it was.
chmod +x "$TEMP_BINARY"
mv "$TEMP_BINARY" "$STAGED_AGENT"
clear_quarantine "$STAGED_AGENT"

if ! "$STAGED_AGENT" --version >/dev/null 2>&1; then
    rm -f "$STAGED_AGENT"
    die "Agent binary installation failed (binary not functional); the existing install was left untouched"
fi

# Back up only once there is a verified, working replacement to install, so a
# failed download or verification leaves no stray backup behind.
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

# Install the CLI binary (gpy). An ABSENT gpy asset is non-fatal: the prompt
# works without it (gpy only powers the CLI and shell completions). A gpy
# asset that downloads but fails verification aborts the install, same as the
# agent: absence is tolerated, tampering is not.
info "Installing GPY CLI (gpy)..."

CLI_BINARY_URL="https://github.com/$REPO/releases/download/$VERSION/$CLI_BINARY_NAME"
TEMP_CLI_BINARY=$(mktemp "${TMPDIR:-/tmp}/gpy-cli.XXXXXX") || die "Failed to create a temp file for the CLI download"
trap 'rm -f "$TEMP_BINARY" "$TEMP_CLI_BINARY" "$STAGED_AGENT" "$STAGED_CLI" 2>/dev/null' EXIT

CLI_DOWNLOADED=0
info "Downloading CLI from $CLI_BINARY_URL..."
if fetch_to "$CLI_BINARY_URL" "$TEMP_CLI_BINARY"; then
    CLI_DOWNLOADED=1
fi

if [ "$CLI_DOWNLOADED" -eq 1 ]; then
    verify_download "$TEMP_CLI_BINARY" "$CLI_BINARY_URL" "$CLI_BINARY_NAME"

    # Stage next to the destination and check it there before it replaces
    # the installed gpy; the CLI stays optional, so a staged binary that
    # cannot run is dropped with a warning and the old gpy is kept.
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
        warn "Downloaded CLI binary is not functional on this host; the existing gpy (if any) was left untouched and shell completions may be unavailable"
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

# Shell files are pinned to the same release tag as the binary so a fresh
# install never pairs a release binary with unreleased shell scripts.
SHELL_FILES_BASE_URL="https://raw.githubusercontent.com/$REPO/$VERSION/$CURRENT_SHELL"

# The EXIT trap now covers every temp path created so far.
TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/gpy-install.XXXXXX") || die "Failed to create a temp directory for downloads"
trap 'rm -f "$TEMP_BINARY" "$TEMP_CLI_BINARY" "$STAGED_AGENT" "$STAGED_CLI" 2>/dev/null; rm -rf "$TEMP_DIR" 2>/dev/null' EXIT

# Per-shell file lists, kept in named variables so
# tests/fish/install_oneline_file_lists.test.fish can check them against the
# files shipped in the repo.
FISH_CORE_FILES="constants.fish debug.fish init.fish ipc.fish renderer.fish util.fish"
FISH_SEGMENT_FILES="clock.fish devtools.fish directory.fish duration.fish git.fish hostname.fish language.fish status.fish username.fish"
FISH_FUNCTION_FILES="fish_prompt.fish"

ZSH_CORE_FILES="constants.zsh init.zsh ipc.zsh renderer.zsh signals.zsh supervisor.zsh"
ZSH_SEGMENT_FILES="clock.zsh directory.zsh duration.zsh git.zsh hostname.zsh language.zsh status.zsh username.zsh"

BASH_CORE_FILES="constants.bash init.bash ipc.bash renderer.bash signals.bash supervisor.bash"
BASH_SEGMENT_FILES="clock.bash directory.bash duration.bash git.bash hostname.bash language.bash status.bash username.bash"

# Download shell files based on shell type
case "$CURRENT_SHELL" in
    fish)
        info "Downloading Fish shell files..."
        for dir in core segments functions conf.d; do
            mkdir -p "$SHELL_CONFIG_DIR/$dir"
        done

        # Core files, the conf.d/gpy_init.fish entry point (sourced by the
        # config.fish block) and the functions below (fish_prompt.fish is the
        # prompt symlink target) are required, unlike the best-effort
        # segments: a failed download stops the install before any rc file is
        # touched, so rc files never point at an incomplete integration.
        for file in $FISH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        fetch_to "https://raw.githubusercontent.com/$REPO/$VERSION/fish/conf.d/gpy_init.fish" "$SHELL_CONFIG_DIR/conf.d/gpy_init.fish" || die "Failed to download gpy_init.fish"

        # Download segments
        for segment in $FISH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # Download functions (required: the prompt symlink targets them)
        for func in $FISH_FUNCTION_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/functions/$func" "$SHELL_CONFIG_DIR/functions/$func" || die "Failed to download $func"
        done

        # Fish autoloads `completions/<command>.fish` only for the command of
        # that name, so the structural completions and the checked-in
        # gpy-dynamic.fish glue (live theme/palette/segment names via
        # `gpy __complete <kind>`) share ONE file, completions/gpy.fish. Without
        # structural completions, gpy.fish is the glue alone. The glue is
        # best-effort: a failed download only loses dynamic-value completions.
        FISH_COMPLETIONS_DIR="$CONFIG_HOME/fish/completions"
        mkdir -p "$FISH_COMPLETIONS_DIR"
        # The glue is never autoloaded as its own file; remove any stray copy.
        rm -f "$FISH_COMPLETIONS_DIR/gpy-dynamic.fish"

        fish_completions_generated=0
        if generate_completions fish "$FISH_COMPLETIONS_DIR/gpy.fish"; then
            fish_completions_generated=1
        fi

        dynamic_glue_tmp="$(mktemp)"
        if fetch_to "$SHELL_FILES_BASE_URL/completions/gpy-dynamic.fish" "$dynamic_glue_tmp"; then
            if [ "$fish_completions_generated" -eq 1 ]; then
                cat "$dynamic_glue_tmp" >>"$FISH_COMPLETIONS_DIR/gpy.fish"
            else
                cat "$dynamic_glue_tmp" >"$FISH_COMPLETIONS_DIR/gpy.fish"
            fi
            success "Installed dynamic completions glue into: $FISH_COMPLETIONS_DIR/gpy.fish"
        else
            warn "Could not download gpy-dynamic.fish"
        fi
        rm -f "$dynamic_glue_tmp"
        ;;

    zsh)
        info "Downloading Zsh shell files..."
        mkdir -p "$SHELL_CONFIG_DIR/core" "$SHELL_CONFIG_DIR/segments"

        # Download gpy.zsh entry point
        fetch_to "$SHELL_FILES_BASE_URL/gpy.zsh" "$SHELL_CONFIG_DIR/gpy.zsh" || die "Failed to download gpy.zsh"

        # Core files are required, unlike the best-effort segments: a failed
        # download stops the install before any rc file is touched.
        for file in $ZSH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        # Download segments
        for segment in $ZSH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # `_gpy` holds the structural completions; the checked-in
        # `_gpy-dynamic` glue fills in dynamic values and delegates to it.
        # Both are best-effort. `zsh/gpy.zsh` adds them to fpath and runs
        # `compdef` on every shell start, so no rc-file change is needed
        # beyond the gpy-init block.
        mkdir -p "$SHELL_CONFIG_DIR/completions"

        generate_completions zsh "$SHELL_CONFIG_DIR/completions/_gpy" || true

        fetch_to "$SHELL_FILES_BASE_URL/completions/_gpy-dynamic" "$SHELL_CONFIG_DIR/completions/_gpy-dynamic" || warn "Could not download _gpy-dynamic"
        ;;

    bash)
        info "Downloading Bash shell files..."
        mkdir -p "$SHELL_CONFIG_DIR/core" "$SHELL_CONFIG_DIR/segments"

        # Download gpy.bash entry point
        fetch_to "$SHELL_FILES_BASE_URL/gpy.bash" "$SHELL_CONFIG_DIR/gpy.bash" || die "Failed to download gpy.bash"

        # Core files are required, unlike the best-effort segments: a failed
        # download stops the install before any rc file is touched.
        for file in $BASH_CORE_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/core/$file" "$SHELL_CONFIG_DIR/core/$file" || die "Failed to download core file: $file"
        done

        # Download segments
        for segment in $BASH_SEGMENT_FILES; do
            fetch_to "$SHELL_FILES_BASE_URL/segments/$segment" "$SHELL_CONFIG_DIR/segments/$segment" || warn "Could not download $segment"
        done

        # `gpy.bash` holds the structural completions; the checked-in
        # `gpy-dynamic.bash` glue fills in dynamic values and delegates to it.
        # Both are best-effort.
        mkdir -p "$SHELL_CONFIG_DIR/completions"

        generate_completions bash "$SHELL_CONFIG_DIR/completions/gpy.bash" || true

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

        # The sourced path is expanded at install time so the block sources
        # wherever SHELL_CONFIG_DIR landed; a literal '~' would miss an
        # XDG_CONFIG_HOME install.
        if append_gpy_block "$FISH_CONFIG" "GPY Prompt Enhancement" "if status is-interactive
    source \"$SHELL_CONFIG_DIR/conf.d/gpy_init.fish\"
end"; then
            success "Added GPY initialization to $FISH_CONFIG"
        else
            success "GPY already configured in $FISH_CONFIG"
        fi

        # Create symlink for fish_prompt
        FISH_PROMPT_TARGET="$FISH_FUNCTIONS_DIR/fish_prompt.fish"
        FISH_PROMPT_SOURCE="$SHELL_CONFIG_DIR/functions/fish_prompt.fish"

        # Move aside anything that is not GPY's own link, foreign symlinks
        # included; `mv` keeps a symlink a symlink.
        if { [ -e "$FISH_PROMPT_TARGET" ] || [ -L "$FISH_PROMPT_TARGET" ]; } \
            && { [ ! -L "$FISH_PROMPT_TARGET" ] || [ "$(readlink "$FISH_PROMPT_TARGET")" != "$FISH_PROMPT_SOURCE" ]; }; then
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
        # zsh reads ${ZDOTDIR:-$HOME}/.zshrc. A file this installer creates
        # is tagged `gpy-created-file` so the uninstaller can delete it again.
        ZSH_RC="${ZDOTDIR:-$HOME}/.zshrc"
        ZSH_RC_NOTE=""
        [ -e "$ZSH_RC" ] || ZSH_RC_NOTE=" (gpy-created-file)"
        mkdir -p "$(dirname "$ZSH_RC")"

        if append_gpy_block "$ZSH_RC" "GPY Prompt Enhancement$ZSH_RC_NOTE" "source \"$SHELL_CONFIG_DIR/gpy.zsh\""; then
            success "Added GPY initialization to $ZSH_RC"
        else
            success "GPY already configured in $ZSH_RC"
        fi
        ;;

    bash)
        # Always write the GPY block to ~/.bashrc (interactive non-login
        # shells read only that), creating it if missing. A file this
        # installer creates is tagged `gpy-created-file` in its block so the
        # uninstaller can delete it again.
        BASH_RC="$HOME/.bashrc"
        BASH_RC_NOTE=""
        [ -e "$BASH_RC" ] || BASH_RC_NOTE=" (gpy-created-file)"

        if append_gpy_block "$BASH_RC" "GPY Prompt Enhancement$BASH_RC_NOTE" "source \"$SHELL_CONFIG_DIR/gpy.bash\""; then
            success "Added GPY initialization to $BASH_RC"
        else
            success "GPY already configured in $BASH_RC"
        fi

        # Login shells read only the first of ~/.bash_profile, ~/.bash_login,
        # ~/.profile. Make that one reach ~/.bashrc without changing which
        # file bash picks: append a loader block only when it does not
        # already mention .bashrc; with no login file at all create
        # ~/.profile (not ~/.bash_profile, which would shadow a later one).
        BASH_LOGIN=""
        for f in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
            if [ -e "$f" ]; then
                BASH_LOGIN="$f"
                break
            fi
        done
        BASH_LOGIN_NOTE=""
        if [ -z "$BASH_LOGIN" ]; then
            BASH_LOGIN="$HOME/.profile"
            BASH_LOGIN_NOTE=" (gpy-created-file)"
        fi
        # shellcheck disable=SC2016 # $HOME must stay literal in the rc file
        if ! grep -q '\.bashrc' "$BASH_LOGIN" 2>/dev/null \
            && append_gpy_block "$BASH_LOGIN" "GPY: load ~/.bashrc in login shells$BASH_LOGIN_NOTE" '[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"'; then
            success "Added ~/.bashrc loader to $BASH_LOGIN"
        fi

        # Warn about Bash version if on macOS. Probe the `bash` binary
        # directly rather than $SHELL: GPY_SHELL can force CURRENT_SHELL=bash
        # while the login shell is something else (e.g. ksh), whose
        # --version output would not parse and could abort under `set -e`
        # after the rc file was already modified.
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
# `show_icons` so a machine without a Nerd Font never renders tofu boxes. It
# reads the user's confirmation from /dev/tty (not this piped stdin), so a
# `curl | sh` install with an attached terminal can still show a sample and
# ask; a fully non-interactive pipe falls back to detection. Non-fatal
# (`|| warn`) so a failure never aborts the install under `set -e`.
info "Configuring prompt icons..."
"$INSTALL_DIR/gpy-agent" init || warn "Icon setup skipped (gpy-agent init failed); using built-in defaults"

# Always run `start`, even if an agent already responds to `status`: `start`
# is idempotent (no-op when a matching version is already running) and is
# the only path that detects and restarts a version-mismatched old daemon.
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
case "$CURRENT_SHELL" in
    fish) echo "   • Config: $FISH_CONFIG" ;;
    zsh) echo "   • Config: $ZSH_RC" ;;
    bash) echo "   • Config: $BASH_RC" ;;
esac
echo ""

info "Next steps:"
case "$CURRENT_SHELL" in
    fish)
        echo "   1. Restart Fish: exec fish"
        echo "   2. Or reload config: source \"$FISH_CONFIG\""
        ;;
    zsh)
        echo "   1. Restart Zsh: exec zsh"
        echo "   2. Or reload config: source \"$ZSH_RC\""
        ;;
    bash)
        echo "   1. Restart Bash: exec bash"
        echo "   2. Or reload config: source \"$BASH_RC\""
        ;;
esac
echo ""

info "Configuration and help:"
echo "   • View config: cat $CONFIG_HOME/gpy/config.toml"
echo "   • Agent commands: gpy-agent --help"
echo "   • Documentation: https://github.com/jpease/gpy"
echo "   • Check agent status: gpy-agent status"
echo "   • View logs: tail -f $CACHE_HOME/gpy/agent.log"
# The uninstall scripts are the only complete removal path. Download-then-run
# rather than `curl | sh`: the script asks for confirmation on stdin, which a
# piped script would feed itself to.
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
}

main "$@"
