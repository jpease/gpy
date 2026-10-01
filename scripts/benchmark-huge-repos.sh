#!/bin/bash
# Benchmark gpy-agent performance on huge repositories
# Usage: ./benchmark-huge-repos.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_PATH="/tmp/huge-repo"

# Build release binary
echo "Building gpy-agent in release mode..."
cd "$PROJECT_ROOT/gpy-agent"
CARGO_BUILD_JOBS=2 cargo build --release --quiet

AGENT="$PROJECT_ROOT/gpy-agent/target/release/gpy-agent"

# Create test repo if it doesn't exist
if [ ! -d "$REPO_PATH" ]; then
    echo "Creating test repository..."
    "$SCRIPT_DIR/create-huge-test-repo.sh" "$REPO_PATH" 10000
fi

echo ""
echo "=== Benchmarking with 10k files ==="
echo ""

# Benchmark WITH sampling (GPY_ENABLE_SAMPLING=1)
# Benchmark WITHOUT sampling (GPY_NO_SAMPLING=1 or default)
hyperfine --warmup 3 --runs 10 \
  --export-markdown /tmp/gpy-benchmark-results.md \
  --export-json /tmp/gpy-benchmark-results.json \
  "GPY_ENABLE_SAMPLING=1 $AGENT oneshot git --cwd $REPO_PATH --format json" \
  "$AGENT oneshot git --cwd $REPO_PATH --format json"

echo ""
echo "Results exported to /tmp/gpy-benchmark-results.{md,json}"
echo ""

# Show results
cat /tmp/gpy-benchmark-results.md
