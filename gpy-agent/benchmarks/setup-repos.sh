#!/bin/bash
# Setup real-world benchmark repositories
#
# This script clones real-world repositories of varying sizes for consistent
# performance testing. These repos represent actual user workloads better than
# synthetic test data.

set -e

BENCH_DIR="${BENCH_DIR:-$HOME/.cache/gpy-benchmarks}"

echo "=== Setting up benchmark repositories in $BENCH_DIR ==="
echo ""

mkdir -p "$BENCH_DIR"
cd "$BENCH_DIR"

# Function to setup a repo for benchmarking
setup_repo() {
    local name=$1
    local url=$2
    local branch=$3
    local commit=$4
    local description=$5

    echo "[$name] $description"

    if [ ! -d "$name" ]; then
        echo "  Cloning..."
        if [ -n "$commit" ]; then
            # Clone and checkout specific commit/tag
            git clone --depth 1 --no-single-branch "$url" "$name" 2>&1 | grep -v "Receiving objects" || true
        else
            # Shallow clone of branch head
            git clone --depth 1 --single-branch --branch "$branch" "$url" "$name" 2>&1 | grep -v "Receiving objects" || true
        fi
    else
        echo "  ✓ Already exists"
    fi

    cd "$name"

    if [ -n "$commit" ]; then
        echo "  Resetting to pinned revision: $commit"
        git fetch --depth 1 origin "refs/tags/$commit:refs/tags/$commit" 2>/dev/null || true
        git fetch --depth 1 origin "$commit" 2>/dev/null || true
        git checkout -f "$commit" 2>/dev/null || {
            echo "  Warning: Could not checkout $commit, using current HEAD"
        }
    else
        echo "  Resetting to branch head: $branch"
        git fetch --depth 1 origin "$branch" >/dev/null 2>&1 || true
        git checkout -f "$branch" >/dev/null 2>&1 || true
        git reset --hard "origin/$branch" >/dev/null 2>&1 || true
    fi

    # Ensure existing repos do not retain local modifications between runs.
    git reset --hard HEAD >/dev/null 2>&1 || true
    git clean -fdx >/dev/null 2>&1 || true

    # Count files
    local files=$(git ls-files | wc -l | tr -d ' ')
    local index_size=$(stat -f%z .git/index 2>/dev/null || stat -c%s .git/index)
    local index_mb=$(echo "scale=2; $index_size/1024/1024" | bc)

    echo "  ✓ $files files, ${index_mb}MB index"
    cd ..
    echo ""
}

# Tiny repo: ~200 files
# Pinned to specific commit for reproducibility
setup_repo \
    "tiny-express" \
    "https://github.com/expressjs/express.git" \
    "master" \
    "4.21.1" \
    "Tiny: Express.js web framework (~200 files)"

# Small repo: ~1.7k files
# Pinned to Redis 7.0.0 for reproducibility
setup_repo \
    "small-redis" \
    "https://github.com/redis/redis.git" \
    "7.0" \
    "7.0.0" \
    "Small: Redis database (~1.7k files)"

# Medium repo: ~10k files
# Use a smaller, more representative repo
setup_repo \
    "medium-kubernetes" \
    "https://github.com/kubernetes/kubernetes.git" \
    "master" \
    "v1.28.0" \
    "Medium: Kubernetes orchestrator (~10k files)"

# Large repo: ~36k files
# Pinned to a specific TensorFlow revision to keep the macOS benchmark
# fixture case-insensitive-safe while still exercising a substantially
# larger worktree than the medium repo.
setup_repo \
    "large-tensorflow" \
    "https://github.com/tensorflow/tensorflow.git" \
    "master" \
    "82da3924133ffa69250caf971d305e9b7fb44b2d" \
    "Large: TensorFlow (~36k files, case-insensitive-safe on macOS)"

echo "=== Benchmark repositories ready ==="
echo ""
echo "Location: $BENCH_DIR"
echo ""
echo "Repositories:"
for repo in tiny-express small-redis medium-kubernetes large-tensorflow; do
    if [ -d "$repo" ]; then
        files=$(cd "$repo" && git ls-files | wc -l | tr -d ' ')
        index_size=$(stat -f%z "$repo/.git/index" 2>/dev/null || stat -c%s "$repo/.git/index")
        index_mb=$(echo "scale=2; $index_size/1024/1024" | bc)
        printf "  %-25s: %6s files, %6s MB index\n" "$repo" "$files" "$index_mb"
    fi
done
