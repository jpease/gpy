#!/bin/bash
# Compare GPY against Powerlevel10k and Starship
#
# Runs identical benchmarks against all three prompt systems for
# apples-to-apples performance comparison.

set -e

BENCH_DIR="${BENCH_DIR:-$HOME/.cache/gpy-benchmarks}"
BENCH_RUNS="${BENCH_RUNS:-5}"
# Ensure at least 2 runs (hyperfine output format changes with 1 run)
[ "$BENCH_RUNS" -lt 2 ] && BENCH_RUNS=2
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GPY_AGENT="$SCRIPT_DIR/../target/release/gpy-agent"

echo "=== Prompt Performance Comparison ==="
echo ""
echo "Testing: GPY vs Powerlevel10k vs Starship"
echo "Repos:   $BENCH_DIR"
echo "Runs:    $BENCH_RUNS per benchmark"
echo ""

# Check prerequisites
command -v starship >/dev/null 2>&1 || {
    echo "⚠️  Starship not found. Install: brew install starship"
    echo "   (Will skip Starship benchmarks)"
    HAS_STARSHIP=false
}
HAS_STARSHIP=true

command -v zsh >/dev/null 2>&1 || {
    echo "⚠️  Zsh not found (needed for p10k)"
    echo "   (Will skip Powerlevel10k benchmarks)"
    HAS_P10K=false
}

# Check if p10k is installed
if [ -d "${ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}/themes/powerlevel10k" ] || \
   [ -f "/opt/homebrew/share/powerlevel10k/powerlevel10k.zsh-theme" ]; then
    HAS_P10K=true
else
    echo "⚠️  Powerlevel10k not found"
    echo "   Install: git clone --depth=1 https://github.com/romkatv/powerlevel10k.git \${ZSH_CUSTOM:-\$HOME/.oh-my-zsh/custom}/themes/powerlevel10k"
    echo "   (Will skip Powerlevel10k benchmarks)"
    HAS_P10K=false
fi

if [ ! -f "$GPY_AGENT" ]; then
    echo "Error: GPY agent not found at $GPY_AGENT"
    echo "Run: cargo build --release"
    exit 1
fi

if [ ! -d "$BENCH_DIR" ]; then
    echo "Error: Benchmark repositories not found at $BENCH_DIR"
    echo "Run: ./benchmarks/setup-repos.sh"
    exit 1
fi

echo ""

# Function to benchmark GPY
bench_gpy() {
    local repo=$1
    hyperfine \
        --warmup 2 \
        --runs "$BENCH_RUNS" \
        --export-json "/tmp/bench-gpy-$repo.json" \
        "bash -c 'cd $BENCH_DIR/$repo && $GPY_AGENT oneshot git --cwd . --format json'" \
        2>&1 | grep "Time (mean" | sed 's/.*Time (mean ± σ):  *//'
}

# Function to benchmark Starship
bench_starship() {
    local repo=$1
    if [ "$HAS_STARSHIP" = true ]; then
        hyperfine \
            --warmup 2 \
            --runs "$BENCH_RUNS" \
            --export-json "/tmp/bench-starship-$repo.json" \
            "bash -c 'cd $BENCH_DIR/$repo && starship module git_status'" \
            2>&1 | grep "Time (mean" | sed 's/.*Time (mean ± σ):  *//'
    else
        echo "N/A"
    fi
}

# Function to benchmark p10k
bench_p10k() {
    local repo=$1
    if [ "$HAS_P10K" = true ]; then
        # p10k is harder to benchmark in isolation since it's a zsh theme
        # We'll benchmark the git status segment specifically
        hyperfine \
            --warmup 2 \
            --runs "$BENCH_RUNS" \
            --export-json "/tmp/bench-p10k-$repo.json" \
            "zsh -c 'cd $BENCH_DIR/$repo && source /opt/homebrew/share/powerlevel10k/powerlevel10k.zsh-theme 2>/dev/null && gitstatus_query'" \
            2>&1 | grep "Time (mean" | sed 's/.*Time (mean ± σ):  *//' || echo "N/A"
    else
        echo "N/A"
    fi
}

# Run benchmarks
REPOS=("tiny-express" "small-redis" "medium-kubernetes" "large-tensorflow")

echo "=== Results ==="
echo ""
printf "%-25s | %-20s | %-20s | %-20s\n" "Repository" "GPY" "Starship" "Powerlevel10k"
printf "%-25s-+-%-20s-+-%-20s-+-%-20s\n" "-------------------------" "--------------------" "--------------------" "--------------------"

for repo in "${REPOS[@]}"; do
    if [ ! -d "$BENCH_DIR/$repo" ]; then
        continue
    fi

    printf "%-25s | " "$repo"

    gpy_result=$(bench_gpy "$repo")
    printf "%-20s | " "$gpy_result"

    starship_result=$(bench_starship "$repo")
    printf "%-20s | " "$starship_result"

    p10k_result=$(bench_p10k "$repo")
    printf "%-20s\n" "$p10k_result"
done

echo ""
echo "=== Analysis ==="
echo ""
echo "Notes:"
echo "- GPY: Full git status via native git subprocess"
echo "- Starship: git_status module only"
echo "- Powerlevel10k: Uses gitstatusd daemon (if available)"
echo ""
echo "All measurements are for git status detection only, not full prompt rendering."
