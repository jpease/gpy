#!/bin/bash
set -e
# Extracted from scripts/competitive-bench.sh to set up benchmark repos

# --- Configuration ---
BENCH_DIR="/tmp/gpy-bench-repos"
REPOS_DIR="$BENCH_DIR"
NORMAL_DIR="$REPOS_DIR/normal-dir"
SMALL_REPO="$REPOS_DIR/small-repo"
LARGE_REPO="$REPOS_DIR/large-repo"
LARGE_REPO_URL="https://github.com/torvalds/linux.git"

# --- Helper Functions ---
step() {
  echo "==> $1"
}

cleanup_repos() {
  if [ -d "$BENCH_DIR" ]; then
    step "Removing existing benchmark repo dir: $BENCH_DIR"
    rm -rf "$BENCH_DIR"
  fi
}

# --- Main Setup ---
setup_repos() {
  if [ -d "$BENCH_DIR" ]; then
    step "Benchmark directory already exists. Skipping setup."
    return
  fi

  step "Setting up benchmark repositories in $BENCH_DIR..."
  mkdir -p "$REPOS_DIR"

  # 1. Normal Directory (no git)
  step "Creating normal directory..."
  mkdir -p "$NORMAL_DIR"
  touch "$NORMAL_DIR/file1.txt"
  touch "$NORMAL_DIR/file2.log"
  mkdir "$NORMAL_DIR/subdir"
  touch "$NORMAL_DIR/subdir/file3.js"

  # 2. Small Git Repository
  step "Creating small git repository..."
  git init "$SMALL_REPO" >/dev/null
  (
    cd "$SMALL_REPO"
    git config user.email "test@example.com"
    git config user.name "Test User"
    for i in $(seq 1 10); do
      echo "commit $i" > "file$i.txt"
      git add .
      git commit -m "Commit $i" >/dev/null
    done
    echo "untracked" > untracked.txt
  )

  # 3. Large Git Repository (shallow clone)
  step "Cloning large git repository (linux kernel)..."
  git clone --depth 1 "$LARGE_REPO_URL" "$LARGE_REPO"

  step "Benchmark repositories are ready."
}

cleanup_repos
setup_repos
