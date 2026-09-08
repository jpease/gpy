#!/bin/bash
# Create a huge git repository for performance testing
# Usage: ./create-huge-test-repo.sh [path] [file_count]

REPO_PATH="${1:-/tmp/huge-repo}"
FILE_COUNT="${2:-100000}"

echo "Creating huge repo at $REPO_PATH with $FILE_COUNT files..."

rm -rf "$REPO_PATH"
mkdir -p "$REPO_PATH"
cd "$REPO_PATH" || exit 1

git init

echo "Generating files..."
# Generate files in batches for speed
for i in $(seq 1 1000 "$FILE_COUNT"); do
    end=$((i + 999))
    if [ "$end" -gt "$FILE_COUNT" ]; then end=$FILE_COUNT; fi

    for j in $(seq "$i" "$end"); do
        echo "content $j" > "file_$j.txt"
    done

    # Stage every 1000 files to keep git happy
    if (( i % 10000 == 1 )); then
        echo "Created $i files..."
        git add .
    fi
done

git add .
git commit -m "Initial huge commit"

# Create some dirty state
echo "dirty" > file_1.txt
echo "dirty" > file_100.txt
echo "untracked" > untracked_file.txt

echo "Done! Repo ready at $REPO_PATH"
