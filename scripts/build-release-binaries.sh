#!/bin/bash
# Build GPY release binaries for all platforms
# This script builds binaries locally for testing before creating a GitHub release

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RESET='\033[0m'

info() { printf "${BLUE}ℹ${RESET} %s\n" "$1"; }
success() { printf "${GREEN}✓${RESET} %s\n" "$1"; }
warn() { printf "${YELLOW}⚠${RESET} %s\n" "$1"; }
error() { printf "${RED}✗${RESET} %s\n" "$1"; }

# Change to gpy-agent directory
cd "$(dirname "$0")/../gpy-agent" || exit 1

info "Building GPY release binaries..."
echo ""

# Detect current platform
CURRENT_OS=$(uname -s | tr '[:upper:]' '[:lower:]')
CURRENT_ARCH=$(uname -m)

info "Current platform: $CURRENT_OS ($CURRENT_ARCH)"
echo ""

# Create bin directory in repo root
mkdir -p ../bin

# Sidecars use the same two-space coreutils format as write_sidecar in
# scripts/package-release.sh, which is what install.sh's verify_checksum reads.
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    error "no SHA-256 tool found (need sha256sum or shasum)"
    exit 1
fi

write_sidecar() {
    local file="$1"
    printf '%s  %s\n' "$(sha256_of "$file")" "$(basename "$file")" >"$file.sha256"
}

# Linux binaries are pinned to the release workflow's glibc floor (#694, #818):
# a plain `cargo build` links the build host's glibc. The floor is read from
# GPY_GLIBC_FLOOR in release.yml so there is one source of truth.
GLIBC_FLOOR=$(sed -n 's/^  GPY_GLIBC_FLOOR: "\([0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' ../.github/workflows/release.yml)
if [ -z "$GLIBC_FLOOR" ]; then
    error "could not read GPY_GLIBC_FLOOR from .github/workflows/release.yml"
    exit 1
fi

# Function to build for a target
build_target() {
    local target=$1
    local output_name=$2

    info "Building for $target..."

    # Check if target is installed
    if ! rustup target list --installed | grep -q "$target"; then
        info "Installing target $target..."
        rustup target add "$target" || {
            warn "Could not install target $target (may require cross-compilation tools)"
            return 1
        }
    fi

    # Build (release-dist: opt-level 3 + thin LTO, see gpy-agent/Cargo.toml).
    # Linux targets use cargo-zigbuild with the glibc-floor suffix, exactly as
    # the release workflow does; zigbuild keeps the output at target/<triple>/.
    local build_cmd=(cargo build --profile release-dist --locked --target "$target")
    case "$target" in
        *-unknown-linux-gnu)
            if ! command -v cargo-zigbuild >/dev/null 2>&1 || ! command -v zig >/dev/null 2>&1; then
                warn "Linux release binaries need cargo-zigbuild and zig (glibc $GLIBC_FLOOR floor, #694); install both or use the release workflow"
                return 1
            fi
            build_cmd=(cargo zigbuild --profile release-dist --locked --target "$target.$GLIBC_FLOOR")
            ;;
    esac
    if "${build_cmd[@]}"; then
        # install.sh needs BOTH the agent and the CLI per platform, each with a
        # .sha256 sidecar (#327, #494), so stage the pair with its sidecars.
        local agent_path="target/$target/release-dist/gpy-agent"
        local cli_path="target/$target/release-dist/gpy"
        local cli_name="gpy-${output_name#gpy-agent-}"
        if [ "$target" = "x86_64-pc-windows-msvc" ]; then
            agent_path="$agent_path.exe"
            cli_path="$cli_path.exe"
            output_name="${output_name}.exe"
            cli_name="${cli_name}.exe"
        fi

        local path name
        for path in "$agent_path" "$cli_path"; do
            if [ ! -f "$path" ]; then
                error "Binary not found at $path"
                return 1
            fi
        done

        case "$target" in
            *-unknown-linux-gnu)
                for path in "$agent_path" "$cli_path"; do
                    if ! ../scripts/check-glibc-floor.sh "$path" "$GLIBC_FLOOR"; then
                        error "$path exceeds the glibc $GLIBC_FLOOR floor; not staging $target"
                        return 1
                    fi
                done
                ;;
        esac

        # release-dist already strips symbols (strip = true), so no manual
        # strip step is needed here.
        for name in "$output_name" "$cli_name"; do
            if [ "$name" = "$output_name" ]; then
                path="$agent_path"
            else
                path="$cli_path"
            fi
            cp "$path" "../bin/$name"
            write_sidecar "../bin/$name"
            success "Built $name ($(du -h "../bin/$name" | cut -f1))"
        done
        return 0
    else
        error "Build failed for $target"
        return 1
    fi
}

# Build for current platform first (guaranteed to work)
echo ""
info "Building for current platform..."
case "$CURRENT_OS" in
    linux)
        case "$CURRENT_ARCH" in
            x86_64)
                build_target "x86_64-unknown-linux-gnu" "gpy-agent-linux-x86_64"
                ;;
            aarch64|arm64)
                build_target "aarch64-unknown-linux-gnu" "gpy-agent-linux-aarch64"
                ;;
        esac
        ;;
    darwin)
        case "$CURRENT_ARCH" in
            x86_64)
                build_target "x86_64-apple-darwin" "gpy-agent-macos-x86_64"
                ;;
            arm64)
                build_target "aarch64-apple-darwin" "gpy-agent-macos-aarch64"
                ;;
        esac
        ;;
esac

# Try to build for other platforms (may require cross-compilation)
echo ""
info "Attempting to build for other platforms (may fail without cross-compilation tools)..."
echo ""

# Linux targets
if [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-x86_64" ]; then
    build_target "x86_64-unknown-linux-gnu" "gpy-agent-linux-x86_64" || warn "Skipped Linux x86_64 (requires cross-compilation)"
fi

if [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-aarch64" ] && [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-arm64" ]; then
    build_target "aarch64-unknown-linux-gnu" "gpy-agent-linux-aarch64" || warn "Skipped Linux aarch64 (requires cross-compilation)"
fi

# macOS targets
if [ "$CURRENT_OS-$CURRENT_ARCH" != "darwin-x86_64" ]; then
    build_target "x86_64-apple-darwin" "gpy-agent-macos-x86_64" || warn "Skipped macOS x86_64 (requires macOS)"
fi

if [ "$CURRENT_OS-$CURRENT_ARCH" != "darwin-arm64" ]; then
    build_target "aarch64-apple-darwin" "gpy-agent-macos-aarch64" || warn "Skipped macOS arm64 (requires macOS with Apple Silicon)"
fi

# Windows target (usually requires Windows or complex cross-compilation)
build_target "x86_64-pc-windows-msvc" "gpy-agent-windows-x86_64" || warn "Skipped Windows (requires Windows or cross-compilation setup)"

# Summary
echo ""
info "Build summary:"
echo ""
ls -lh ../bin/ 2>/dev/null || true

echo ""
success "Binaries available in bin/ directory"
echo ""

info "Next steps:"
echo "  1. Install the host platform's bundle from bin/:"
echo "     ./install.sh"
echo ""
echo "  2. When ready for release, create and push a tag:"
echo "     git tag v0.1.0"
echo "     git push origin v0.1.0"
echo ""
echo "  3. GitHub Actions will automatically:"
echo "     - Build all platform binaries"
echo "     - Run tests"
echo "     - Create a GitHub release"
echo "     - Upload all binaries"
echo ""

info "Cross-compilation tips:"
echo "  • Linux: Install zig and cargo-zigbuild (no gcc cross toolchain needed)"
echo "  • Windows: Install mingw-w64 or use cross"
echo "  • macOS: Requires macOS SDK and proper toolchain"
echo "  • Easiest: Use GitHub Actions (already configured!)"
echo ""
