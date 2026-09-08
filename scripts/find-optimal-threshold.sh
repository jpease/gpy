#!/bin/bash
# Find the optimal sampling threshold by benchmarking across repo sizes
# Usage: ./find-optimal-threshold.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_REPOS_DIR="/tmp/gpy-test-repos"
RESULTS_FILE="/tmp/gpy-threshold-analysis.md"

# Build release binary
echo "Building gpy-agent in release mode..."
cd "$PROJECT_ROOT/gpy-agent"
RUSTC_WRAPPER="" CARGO_BUILD_JOBS=2 cargo build --release --quiet

AGENT="$PROJECT_ROOT/gpy-agent/target/release/gpy-agent"

# Create pruned test repos if they don't exist
if [ ! -d "$TEST_REPOS_DIR" ]; then
    echo "Creating test repositories..."
    "$SCRIPT_DIR/create-pruned-linux-repos.sh"
fi

echo ""
echo "=== Benchmarking to Find Optimal Sampling Threshold ==="
echo ""

# Initialize results file
cat > "$RESULTS_FILE" << 'EOF'
# GPY Sampling Threshold Analysis

Testing performance across different repository sizes to find the optimal threshold.

## Methodology

- Real Linux kernel repos pruned to various sizes (preserves structure and .gitignore)
- Each test: 10 runs with 3 warmup runs
- Comparison: Full scan vs Sampling (1,000 sample size)
- Metric: Mean execution time in milliseconds

## Results

| Repo Size | Full Scan (ms) | Sampling (ms) | Speedup | Recommendation |
|----------:|---------------:|-------------:|---------:|:---------------|
EOF

# Test each repo size
SIZES=(1000 2500 5000 7500 10000 15000 20000 30000 40000)

for SIZE in "${SIZES[@]}"; do
    REPO_PATH="$TEST_REPOS_DIR/linux-${SIZE}"

    if [ ! -d "$REPO_PATH" ]; then
        echo "⚠ Skipping $SIZE (repo not found)"
        continue
    fi

    echo "Testing repo with $SIZE files..."

    # Benchmark full scan
    hyperfine --warmup 3 --runs 10 --export-json /tmp/full.json \
        "GPY_NO_SAMPLING=1 $AGENT oneshot git --cwd $REPO_PATH --format json" >/dev/null 2>&1
    FULL_TIME=$(jq -r '.results[0].mean * 1000' /tmp/full.json)

    # Benchmark with sampling
    hyperfine --warmup 3 --runs 10 --export-json /tmp/sample.json \
        "GPY_ENABLE_SAMPLING=1 $AGENT oneshot git --cwd $REPO_PATH --format json" >/dev/null 2>&1
    SAMPLE_TIME=$(jq -r '.results[0].mean * 1000' /tmp/sample.json)

    # Calculate speedup
    SPEEDUP=$(echo "scale=2; $FULL_TIME / $SAMPLE_TIME" | bc)

    # Determine recommendation
    if (( $(echo "$SPEEDUP > 1.1" | bc -l) )); then
        RECOMMENDATION="✓ Enable sampling"
    elif (( $(echo "$SPEEDUP < 0.95" | bc -l) )); then
        RECOMMENDATION="✗ Disable sampling (slower)"
    else
        RECOMMENDATION="~ Neutral (< 10% difference)"
    fi

    # Format times to 1 decimal place
    FULL_TIME_FMT=$(printf "%.1f" "$FULL_TIME")
    SAMPLE_TIME_FMT=$(printf "%.1f" "$SAMPLE_TIME")

    echo "  Full: ${FULL_TIME_FMT}ms, Sampling: ${SAMPLE_TIME_FMT}ms, Speedup: ${SPEEDUP}x"

    # Append to results
    printf "| %'d | %.1f | %.1f | %.2fx | %s |\n" \
        "$SIZE" "$FULL_TIME" "$SAMPLE_TIME" "$SPEEDUP" "$RECOMMENDATION" >> "$RESULTS_FILE"
done

# Add full Linux kernel
REPO_PATH="$TEST_REPOS_DIR/linux-full"
if [ -d "$REPO_PATH" ]; then
    SIZE=$(cd "$REPO_PATH" && git ls-files | wc -l | tr -d ' ')
    echo "Testing full Linux kernel ($SIZE files)..."

    hyperfine --warmup 3 --runs 10 --export-json /tmp/full.json \
        "GPY_NO_SAMPLING=1 $AGENT oneshot git --cwd $REPO_PATH --format json" >/dev/null 2>&1
    FULL_TIME=$(jq -r '.results[0].mean * 1000' /tmp/full.json)

    hyperfine --warmup 3 --runs 10 --export-json /tmp/sample.json \
        "GPY_ENABLE_SAMPLING=1 $AGENT oneshot git --cwd $REPO_PATH --format json" >/dev/null 2>&1
    SAMPLE_TIME=$(jq -r '.results[0].mean * 1000' /tmp/sample.json)

    SPEEDUP=$(echo "scale=2; $FULL_TIME / $SAMPLE_TIME" | bc)

    if (( $(echo "$SPEEDUP > 1.1" | bc -l) )); then
        RECOMMENDATION="✓ Enable sampling"
    elif (( $(echo "$SPEEDUP < 0.95" | bc -l) )); then
        RECOMMENDATION="✗ Disable sampling (slower)"
    else
        RECOMMENDATION="~ Neutral"
    fi

    FULL_TIME_FMT=$(printf "%.1f" "$FULL_TIME")
    SAMPLE_TIME_FMT=$(printf "%.1f" "$SAMPLE_TIME")

    echo "  Full: ${FULL_TIME_FMT}ms, Sampling: ${SAMPLE_TIME_FMT}ms, Speedup: ${SPEEDUP}x"

    printf "| %'d | %.1f | %.1f | %.2fx | %s |\n" \
        "$SIZE" "$FULL_TIME" "$SAMPLE_TIME" "$SPEEDUP" "$RECOMMENDATION" >> "$RESULTS_FILE"
fi

# Add analysis and recommendation
cat >> "$RESULTS_FILE" << 'EOF'

## Analysis

The optimal threshold is the point where:
1. Sampling becomes faster than full scan (speedup > 1.0)
2. The performance gain is significant (speedup > 1.1 = 10% improvement)

**Recommended threshold:** Based on the crossover point in the data above.

EOF

echo ""
echo "=== Analysis Complete ==="
echo ""
cat "$RESULTS_FILE"
echo ""
echo "Full results saved to: $RESULTS_FILE"
