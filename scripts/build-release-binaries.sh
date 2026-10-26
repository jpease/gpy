#!/bin/bash
# Build GPY release binaries for every platform the host can target into bin/,
# for local testing before a tagged release.

set -e

info() { printf "%s\n" "$1"; }
success() { printf "ok: %s\n" "$1"; }
warn() { printf "warn: %s\n" "$1"; }
error() { printf "FAIL: %s\n" "$1"; }

cd "$(dirname "$0")/../gpy-agent" || exit 1

info "Building GPY release binaries..."
echo ""

CURRENT_OS=$(uname -s | tr '[:upper:]' '[:lower:]')
CURRENT_ARCH=$(uname -m)

info "Current platform: $CURRENT_OS ($CURRENT_ARCH)"
echo ""

mkdir -p ../bin

# Same sidecar format as scripts/package-release.sh; install.sh verifies it.
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

# A plain `cargo build` links the host glibc; pin Linux to release.yml's floor.
GLIBC_FLOOR=$(sed -n 's/^  GPY_GLIBC_FLOOR: "\([0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' ../.github/workflows/release.yml)
if [ -z "$GLIBC_FLOOR" ]; then
    error "could not read GPY_GLIBC_FLOOR from .github/workflows/release.yml"
    exit 1
fi

build_target() {
    local target=$1
    local output_name=$2

    info "Building for $target..."

    if ! rustup target list --installed | grep -q "$target"; then
        info "Installing target $target..."
        rustup target add "$target" || {
            warn "Could not install target $target (may require cross-compilation tools)"
            return 1
        }
    fi

    # Linux: cargo-zigbuild with the glibc-floor suffix, as in release.yml.
    local build_cmd=(cargo build --profile release-dist --locked --target "$target")
    case "$target" in
        *-unknown-linux-gnu)
            if ! command -v cargo-zigbuild >/dev/null 2>&1 || ! command -v zig >/dev/null 2>&1; then
                warn "Linux release binaries need cargo-zigbuild and zig (glibc $GLIBC_FLOOR floor); install both or use the release workflow"
                return 1
            fi
            build_cmd=(cargo zigbuild --profile release-dist --locked --target "$target.$GLIBC_FLOOR")
            ;;
    esac
    if "${build_cmd[@]}"; then
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

# Host platform first.
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

echo ""
info "Attempting to build for other platforms (may fail without cross-compilation tools)..."
echo ""

if [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-x86_64" ]; then
    build_target "x86_64-unknown-linux-gnu" "gpy-agent-linux-x86_64" || warn "Skipped Linux x86_64 (requires cross-compilation)"
fi

if [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-aarch64" ] && [ "$CURRENT_OS-$CURRENT_ARCH" != "linux-arm64" ]; then
    build_target "aarch64-unknown-linux-gnu" "gpy-agent-linux-aarch64" || warn "Skipped Linux aarch64 (requires cross-compilation)"
fi

if [ "$CURRENT_OS-$CURRENT_ARCH" != "darwin-x86_64" ]; then
    build_target "x86_64-apple-darwin" "gpy-agent-macos-x86_64" || warn "Skipped macOS x86_64 (requires macOS)"
fi

if [ "$CURRENT_OS-$CURRENT_ARCH" != "darwin-arm64" ]; then
    build_target "aarch64-apple-darwin" "gpy-agent-macos-aarch64" || warn "Skipped macOS arm64 (requires macOS with Apple Silicon)"
fi

build_target "x86_64-pc-windows-msvc" "gpy-agent-windows-x86_64" || warn "Skipped Windows (requires Windows or cross-compilation setup)"

echo ""
info "Build summary:"
echo ""
ls -lh ../bin/ 2>/dev/null || true

echo ""
success "Binaries available in bin/ directory"
info "Run ./install.sh to install the host platform bundle"
