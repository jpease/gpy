#!/bin/bash
# Run performance benchmarks on real-world repositories
#
# Usage:
#   ./benchmarks/run.sh                    # Run all benchmarks
#   ./benchmarks/run.sh --baseline         # Save as baseline for comparison
#   ./benchmarks/run.sh --compare          # Compare against baseline
#
# Environment variables:
#   BENCH_DIR    - Where benchmark repos are stored (default: ~/.cache/gpy-benchmarks)
#   BENCH_RUNS   - Number of runs per benchmark (default: 5)
#   BENCH_SCOPE  - Benchmark scope: benchmark-mode (default) or user-config

set -e

BENCH_DIR="${BENCH_DIR:-$HOME/.cache/gpy-benchmarks}"
BENCH_RUNS="${BENCH_RUNS:-5}"
BENCH_SCOPE="${BENCH_SCOPE:-benchmark-mode}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT="$SCRIPT_DIR/../target/release/gpy-agent"
BASELINE_FILE="$SCRIPT_DIR/baseline.json"

MODE="run"
if [ "$1" = "--baseline" ]; then
    MODE="baseline"
elif [ "$1" = "--compare" ]; then
    MODE="compare"
fi

# Check if benchmark repos exist
if [ ! -d "$BENCH_DIR" ]; then
    echo "Error: Benchmark repositories not found at $BENCH_DIR"
    echo "Run: ./benchmarks/setup-repos.sh"
    exit 1
fi

# Check if agent binary exists
if [ ! -f "$AGENT" ]; then
    echo "Error: Agent binary not found at $AGENT"
    echo "Run: cargo build --release"
    exit 1
fi

echo "=== GPY Agent Performance Benchmarks ==="
echo ""
echo "Agent:    $AGENT"
echo "Repos:    $BENCH_DIR"
echo "Runs:     $BENCH_RUNS per benchmark"
echo "Mode:     $MODE"
echo "Scope:    $BENCH_SCOPE"
echo "Kernel:   $(uname -sr)"
echo ""

# Function to run a single benchmark
run_benchmark() {
    local name=$1
    local repo="$BENCH_DIR/$name"

    if [ ! -d "$repo" ]; then
        echo "  ⚠️  Repo not found: $name"
        return
    fi

    local files=$(cd "$repo" && git ls-files | wc -l | tr -d ' ')
    local index_size=$(stat -f%z "$repo/.git/index" 2>/dev/null || stat -c%s "$repo/.git/index")
    local index_mb=$(echo "scale=2; $index_size/1024/1024" | bc)

    printf "%-25s (%6s files, %5s MB): " "$name" "$files" "$index_mb"

    local bench_flags=""
    if [ "$BENCH_SCOPE" = "benchmark-mode" ]; then
        bench_flags=" --benchmark-mode"
    fi

    # Run hyperfine and capture result
    local result=$(hyperfine \
        --warmup 2 \
        --runs "$BENCH_RUNS" \
        --export-json /tmp/bench-$name.json \
        "bash -c 'cd $repo && $AGENT oneshot git --cwd . --format json$bench_flags'" \
        2>&1 | grep "Time (mean" | sed 's/.*Time (mean ± σ):  *//')

    echo "$result"

    # Store result
    if [ "$MODE" = "baseline" ]; then
        echo "  Saving to baseline..."
    fi
}

# Run all benchmarks
echo "=== Running Benchmarks ==="
echo ""

REPOS=(
    "tiny-express"
    "small-redis"
    "medium-kubernetes"
    "large-tensorflow"
)

for repo in "${REPOS[@]}"; do
    run_benchmark "$repo"
done

echo ""

if [ "$MODE" = "baseline" ]; then
    # Combine results into baseline file
    echo "{"  > "$BASELINE_FILE"
    echo "  \"timestamp\": \"$(date -u +"%Y-%m-%dT%H:%M:%SZ")\"," >> "$BASELINE_FILE"
    echo "  \"git_commit\": \"$(git rev-parse HEAD)\"," >> "$BASELINE_FILE"
    echo "  \"scope\": \"$BENCH_SCOPE\"," >> "$BASELINE_FILE"
    echo "  \"kernel\": \"$(uname -sr)\"," >> "$BASELINE_FILE"
    echo "  \"benchmarks\": {" >> "$BASELINE_FILE"

    first=true
    for repo in "${REPOS[@]}"; do
        if [ -f "/tmp/bench-$repo.json" ]; then
            if [ "$first" = false ]; then
                echo "," >> "$BASELINE_FILE"
            fi
            first=false

            mean=$(jq -r '.results[0].mean' "/tmp/bench-$repo.json")
            stddev=$(jq -r '.results[0].stddev' "/tmp/bench-$repo.json")

            echo "    \"$repo\": {" >> "$BASELINE_FILE"
            echo "      \"mean\": $mean," >> "$BASELINE_FILE"
            echo "      \"stddev\": $stddev" >> "$BASELINE_FILE"
            echo -n "    }" >> "$BASELINE_FILE"
        fi
    done

    echo "" >> "$BASELINE_FILE"
    echo "  }" >> "$BASELINE_FILE"
    echo "}" >> "$BASELINE_FILE"

    echo "✅ Baseline saved to: $BASELINE_FILE"

elif [ "$MODE" = "compare" ]; then
    if [ ! -f "$BASELINE_FILE" ]; then
        echo "Error: No baseline found at $BASELINE_FILE"
        echo "Run: ./benchmarks/run.sh --baseline"
        exit 1
    fi

    echo "=== Comparison with Baseline ==="
    echo ""

    baseline_commit=$(jq -r '.git_commit' "$BASELINE_FILE")
    baseline_date=$(jq -r '.timestamp' "$BASELINE_FILE")
    baseline_scope=$(jq -r '.scope // "unknown"' "$BASELINE_FILE")

    echo "Baseline: commit $baseline_commit ($baseline_date)"
    echo "Baseline scope: $baseline_scope"
    if [ "$baseline_scope" != "$BENCH_SCOPE" ]; then
        echo "⚠️  Scope mismatch: current run uses '$BENCH_SCOPE' but baseline uses '$baseline_scope'"
        echo "    Compare results cautiously or regenerate the baseline with a matching scope."
    fi
    echo ""

    for repo in "${REPOS[@]}"; do
        if [ -f "/tmp/bench-$repo.json" ]; then
            current_mean=$(jq -r '.results[0].mean' "/tmp/bench-$repo.json")
            baseline_mean=$(jq -r ".benchmarks.\"$repo\".mean" "$BASELINE_FILE")

            if [ "$baseline_mean" != "null" ]; then
                change=$(echo "scale=1; ($current_mean - $baseline_mean) / $baseline_mean * 100" | bc)
                current_ms=$(echo "scale=1; $current_mean * 1000" | bc)
                baseline_ms=$(echo "scale=1; $baseline_mean * 1000" | bc)

                if (( $(echo "$change > 10" | bc -l) )); then
                    status="🔴 REGRESSION"
                elif (( $(echo "$change < -10" | bc -l) )); then
                    status="🟢 IMPROVEMENT"
                else
                    status="✓ No change"
                fi

                printf "  %-25s: %6.1f ms → %6.1f ms (%+.1f%%) %s\n" \
                    "$repo" "$baseline_ms" "$current_ms" "$change" "$status"
            fi
        fi
    done
fi

echo ""
echo "=== Benchmark Complete ==="
