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

set -euo pipefail

BENCH_DIR="${BENCH_DIR:-$HOME/.cache/gpy-benchmarks}"
BENCH_RUNS="${BENCH_RUNS:-5}"
BENCH_SCOPE="${BENCH_SCOPE:-benchmark-mode}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT="$SCRIPT_DIR/../target/release/gpy-agent"
BASELINE_FILE="$SCRIPT_DIR/baseline.json"

MODE="run"
if [ "${1:-}" = "--baseline" ]; then
    MODE="baseline"
elif [ "${1:-}" = "--compare" ]; then
    MODE="compare"
fi

if [ "$MODE" = "compare" ] && [ ! -f "$BASELINE_FILE" ]; then
    echo "Error: No baseline found at $BASELINE_FILE"
    echo "Run: ./benchmarks/run.sh --baseline"
    exit 1
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

REPOS=(
    "tiny-express"
    "small-redis"
    "medium-kubernetes"
    "large-tensorflow"
)

# Print "<mean> <stddev>" in seconds from a hyperfine export (1.x and 2.x schemas).
bench_stat() {
    jq -r '.results[0] | (.summary.time_wall_clock // .) | "\(.mean) \(.stddev // 0)"' "$TMP_DIR/$1.json"
}

# Run hyperfine for one repo and write its JSON export to $TMP_DIR/<name>.json.
run_benchmark() {
    local name=$1
    local repo="$BENCH_DIR/$name"

    if [ ! -d "$repo" ]; then
        echo "Error: repo not found: $name" >&2
        exit 1
    fi

    local files index_size index_mb
    files=$(cd "$repo" && git ls-files | wc -l | tr -d ' ')
    index_size=$(stat -f%z "$repo/.git/index" 2>/dev/null || stat -c%s "$repo/.git/index")
    index_mb=$(echo "scale=2; $index_size/1024/1024" | bc)

    printf "%-25s (%6s files, %5s MB): " "$name" "$files" "$index_mb"

    local bench_flags=()
    if [ "$BENCH_SCOPE" = "benchmark-mode" ]; then
        bench_flags=(--benchmark-mode)
    fi

    local cmd
    cmd=$(printf 'cd %q && %q oneshot git --cwd . --format json %s' \
        "$repo" "$AGENT" "${bench_flags[*]:-}")

    hyperfine \
        --warmup 2 \
        --runs "$BENCH_RUNS" \
        --export-json "$TMP_DIR/$name.json" \
        --shell bash \
        "$cmd" >"$TMP_DIR/$name.out" 2>&1
    local mean stddev
    read -r mean stddev < <(bench_stat "$name")
    printf '%.1f ms ± %.1f ms\n' "$(echo "$mean * 1000" | bc -l)" "$(echo "$stddev * 1000" | bc -l)"
}

echo "=== Running Benchmarks ==="
echo ""

for repo in "${REPOS[@]}"; do
    run_benchmark "$repo"
done

echo ""

if [ "$MODE" = "baseline" ]; then
    for repo in "${REPOS[@]}"; do
        read -r mean stddev < <(bench_stat "$repo")
        jq -n --arg name "$repo" --argjson mean "$mean" --argjson stddev "$stddev" \
            '{($name): {mean: $mean, stddev: $stddev}}'
    done | jq -s \
        --arg ts "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
        --arg commit "$(git -C "$SCRIPT_DIR" rev-parse HEAD)" \
        --arg scope "$BENCH_SCOPE" \
        --arg kernel "$(uname -sr)" \
        '{timestamp: $ts, git_commit: $commit, scope: $scope, kernel: $kernel, benchmarks: add}' \
        >"$BASELINE_FILE"

    echo "Baseline saved to: $BASELINE_FILE"

elif [ "$MODE" = "compare" ]; then
    echo "=== Comparison with Baseline ==="
    echo ""

    baseline_commit=$(jq -r '.git_commit' "$BASELINE_FILE")
    baseline_date=$(jq -r '.timestamp' "$BASELINE_FILE")
    baseline_scope=$(jq -r '.scope // "unknown"' "$BASELINE_FILE")

    echo "Baseline: commit $baseline_commit ($baseline_date)"
    echo "Baseline scope: $baseline_scope"
    if [ "$baseline_scope" != "$BENCH_SCOPE" ]; then
        echo "Warning: scope mismatch: current run uses '$BENCH_SCOPE' but baseline uses '$baseline_scope'"
        echo "    Compare results cautiously or regenerate the baseline with a matching scope."
    fi
    echo ""

    missing=0
    for repo in "${REPOS[@]}"; do
        read -r current_mean _ < <(bench_stat "$repo")
        baseline_mean=$(jq -r --arg name "$repo" '.benchmarks[$name].mean // "null"' "$BASELINE_FILE")

        if [ "$baseline_mean" = "null" ]; then
            printf "  %-25s: MISSING from baseline\n" "$repo"
            missing=1
            continue
        fi

        change=$(echo "scale=1; ($current_mean - $baseline_mean) / $baseline_mean * 100" | bc)
        current_ms=$(echo "scale=1; $current_mean * 1000" | bc)
        baseline_ms=$(echo "scale=1; $baseline_mean * 1000" | bc)

        if (( $(echo "$change > 10" | bc -l) )); then
            status="REGRESSION"
        elif (( $(echo "$change < -10" | bc -l) )); then
            status="IMPROVEMENT"
        else
            status="no change"
        fi

        printf "  %-25s: %6.1f ms -> %6.1f ms (%+.1f%%) %s\n" \
            "$repo" "$baseline_ms" "$current_ms" "$change" "$status"
    done

    if [ "$missing" -ne 0 ]; then
        echo ""
        echo "Error: baseline is missing entries; regenerate with: ./benchmarks/run.sh --baseline" >&2
        exit 1
    fi
fi

echo ""
echo "=== Benchmark Complete ==="
