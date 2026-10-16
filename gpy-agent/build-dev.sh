#!/bin/bash
# Development build script that uses system libgit2 for faster builds
set -e

echo "🔨 GPY Development Build (System libgit2)"
echo "========================================="

# Force use of system libgit2 for faster development builds
export LIBGIT2_NO_VENDOR=1

echo "📦 Using system libgit2 for faster builds..."

# Parse arguments
RUN_CLIPPY=false
RUN_COMPREHENSIVE_CLIPPY=false
RUN_RELEASE=false
SKIP_CLIPPY=false
BUILD_ARGS=()

for arg in "$@"; do
    case $arg in
        --clippy)
            RUN_CLIPPY=true
            ;;
        --comprehensive-clippy)
            RUN_COMPREHENSIVE_CLIPPY=true
            ;;
        --no-clippy)
            SKIP_CLIPPY=true
            ;;
        --release)
            RUN_RELEASE=true
            BUILD_ARGS+=("$arg")
            ;;
        *)
            BUILD_ARGS+=("$arg")
            ;;
    esac
done

# Build with system libgit2
RUSTFLAGS="" cargo build "${BUILD_ARGS[@]}"

# Run clippy if requested and not skipped
if [[ "$SKIP_CLIPPY" == "false" ]]; then
    if [[ "$RUN_COMPREHENSIVE_CLIPPY" == "true" ]]; then
        echo "🔍 Running comprehensive clippy checks..."
        # Use the dedicated script for full clippy configuration
        ./gpy-clippy.sh
    elif [[ "$RUN_CLIPPY" == "true" ]]; then
        echo "🔍 Running basic clippy checks..."
        # Use minimal clippy flags to avoid command line length issues
        cargo clippy -- -D warnings
    fi
fi

echo "✅ Development build complete!"
echo "⚡ Fast build using system libgit2 (requires libgit2-dev installed)"
echo "📊 Check binary size: ls -lh target/debug/gpy-agent"
