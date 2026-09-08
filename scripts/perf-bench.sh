#!/bin/bash
# Quick performance benchmark for A/B testing
# Usage: ./scripts/perf-bench.sh [output-name]
#
# Runs 50 iterations across 3 test scenarios:
# - normal-dir: Non-git directory
# - small-repo: ~240 files
# - large-repo: ~5000 files
#
# Requires: hyperfine, test repos in /tmp/gpy-bench-repos/

set -e

OUTPUT_NAME="${1:-benchmark}"
OUTPUT_FILE="/tmp/${OUTPUT_NAME}.json"

cd "$(dirname "$0")/.."

# Check if test repos exist
if [ ! -d "/tmp/gpy-bench-repos/normal-dir" ]; then
    echo "Error: Test repositories not found in /tmp/gpy-bench-repos/"
    echo "Run: ./scripts/setup_bench_repos.sh"
    exit 1
fi

# Check if hyperfine is available
if ! command -v hyperfine &> /dev/null; then
    echo "Error: hyperfine not found"
    echo "Install: brew install hyperfine"
    exit 1
fi

echo "Running benchmark: $OUTPUT_NAME"
echo "Output will be saved to: $OUTPUT_FILE"
echo ""

hyperfine --warmup 5 --runs 50 -N \
  "./gpy-agent/target/release/gpy-agent oneshot git --cwd /tmp/gpy-bench-repos/normal-dir --format json" \
  "./gpy-agent/target/release/gpy-agent oneshot git --cwd /tmp/gpy-bench-repos/small-repo --format json" \
  "./gpy-agent/target/release/gpy-agent oneshot git --cwd /tmp/gpy-bench-repos/large-repo --format json" \
  --export-json "$OUTPUT_FILE"

echo ""
echo "Results saved to: $OUTPUT_FILE"
echo ""
echo "To compare with baseline:"
echo "  hyperfine-compare baseline.json $OUTPUT_FILE"
