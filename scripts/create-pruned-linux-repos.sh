#!/bin/bash
# Create pruned versions of the Linux kernel for threshold benchmarking
# Usage: ./create-pruned-linux-repos.sh

set -e

LINUX_SRC="/tmp/linux"
TEST_REPOS_DIR="/tmp/gpy-test-repos"

echo "=== Creating pruned Linux kernel test repositories ==="
echo ""

# Clone Linux kernel if not present
if [ ! -d "$LINUX_SRC" ]; then
    echo "Cloning Linux kernel v6.6 (this will take a few minutes)..."
    git clone --depth 1 --branch v6.6 https://github.com/torvalds/linux.git "$LINUX_SRC"
else
    echo "Using existing Linux kernel at $LINUX_SRC"
fi

# Get total file count
cd "$LINUX_SRC"
TOTAL_FILES=$(git ls-files | wc -l | tr -d ' ')
echo "Total files in Linux kernel: $TOTAL_FILES"
echo ""

# Test sizes we want: 1k, 2.5k, 5k, 7.5k, 10k, 15k, 20k, 30k, 40k, full
SIZES=(1000 2500 5000 7500 10000 15000 20000 30000 40000 "$TOTAL_FILES")

mkdir -p "$TEST_REPOS_DIR"

for SIZE in "${SIZES[@]}"; do
    REPO_PATH="$TEST_REPOS_DIR/linux-${SIZE}"

    if [ "$SIZE" -eq "$TOTAL_FILES" ]; then
        echo "Creating full Linux kernel repo ($SIZE files)..."
        REPO_PATH="$TEST_REPOS_DIR/linux-full"
    else
        echo "Creating pruned Linux kernel repo ($SIZE files)..."
    fi

    # Remove existing and create fresh copy
    rm -rf "$REPO_PATH"
    # Copy excluding sockets to avoid warnings
    rsync -a --exclude='.git/fsmonitor--daemon.ipc' "$LINUX_SRC/" "$REPO_PATH/"
    cd "$REPO_PATH"

    # Create a proper branch (Linux source is in detached HEAD)
    git checkout -b test-branch 2>/dev/null || git checkout test-branch

    if [ "$SIZE" -lt "$TOTAL_FILES" ]; then
        # Get list of files to DELETE (much faster to delete than to keep)
        FILES_TO_KEEP=$SIZE

        # Randomly select files to keep (cross-platform: use sort -R instead of shuf)
        git ls-files | sort -R | head -n "$FILES_TO_KEEP" > /tmp/files_to_keep.txt

        # Get list of files to DELETE (everything not in keep list)
        git ls-files | grep -Fxv -f /tmp/files_to_keep.txt > /tmp/files_to_delete.txt

        # Delete files in batches using xargs (MUCH faster)
        if [ -s /tmp/files_to_delete.txt ]; then
            cat /tmp/files_to_delete.txt | xargs rm -f
        fi

        # Clean up empty directories
        find . -type d -empty -delete 2>/dev/null || true

        # Re-add everything and create a new clean commit
        git add -A
        git commit --amend -m "Pruned Linux kernel to $SIZE files for testing" --no-verify
    fi

    # Create some realistic dirty state (modify 5 files, add 2 untracked)
    MODIFIED_COUNT=0
    git ls-files | sort -R | head -n 5 | while read -r file; do
        if [ -f "$file" ]; then
            echo "// Modified for testing" >> "$file"
            MODIFIED_COUNT=$((MODIFIED_COUNT + 1))
        fi
    done

    echo "untracked_test_file_1.txt" > untracked_1.txt
    echo "untracked_test_file_2.txt" > untracked_2.txt

    # Verify final count
    ACTUAL_COUNT=$(git ls-files | wc -l | tr -d ' ')
    echo "  ✓ Created: $REPO_PATH ($ACTUAL_COUNT files, 5 modified, 2 untracked)"
    echo ""
done

echo "=== Done! Test repositories created in $TEST_REPOS_DIR ==="
echo ""
echo "Available test repos:"
ls -1 "$TEST_REPOS_DIR"
