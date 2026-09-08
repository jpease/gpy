#!/usr/bin/env bash
set -e

REPO_PATH="${1:-/tmp/huge-repo}"
COUNT="${2:-50}"
# Resolve absolute path to gpy binary
GPY_BIN="$(pwd)/gpy-agent/target/release/gpy"

if [ ! -f "$GPY_BIN" ]; then
    echo "Error: $GPY_BIN not found. Please run 'cargo build --release --bin gpy' in gpy-agent."
    exit 1
fi

# Ensure agent is running (using the same binary)
"$GPY_BIN" start >/dev/null 2>&1 || true

echo "Benchmarking IPC variance on $REPO_PATH ($COUNT runs)..."
echo "Warmup..."
cd "$REPO_PATH"
"$GPY_BIN" debug prompt >/dev/null || echo "Warmup 1 timed out (expected for cold cache)"
"$GPY_BIN" debug prompt >/dev/null || echo "Warmup 2 timed out"

echo "Collecting data..."
TIMINGS_FILE=$(mktemp)

for _ in $(seq 1 "$COUNT"); do
    # Run debug prompt and capture output
    OUTPUT=$("$GPY_BIN" debug prompt)

    # Handle microsecond conversion if needed (rough heuristic: if no decimal, it might be us?)
    # Rust Debug format: "160.29µs", "23.00ms".
    # If ends in µs, divide by 1000.

    RAW_GIT=$(echo "$OUTPUT" | grep "Git Status:" | awk '{print $3}')
    if [[ "$RAW_GIT" == *"µs" ]]; then
        GIT_VAL="${RAW_GIT%µs}"
        GIT_MS=$(echo "scale=4; $GIT_VAL / 1000" | bc)
    else
        GIT_MS="${RAW_GIT%ms}"
    fi

    RAW_LANG=$(echo "$OUTPUT" | grep "Language Detection:" | awk '{print $3}')
    if [[ "$RAW_LANG" == *"µs" ]]; then
        LANG_VAL="${RAW_LANG%µs}"
        LANG_MS=$(echo "scale=4; $LANG_VAL / 1000" | bc)
    else
        LANG_MS="${RAW_LANG%ms}"
    fi

    RAW_TOTAL=$(echo "$OUTPUT" | grep "Total Render Estimate:" | awk '{print $4}')
    if [[ "$RAW_TOTAL" == *"µs" ]]; then
        TOTAL_VAL="${RAW_TOTAL%µs}"
        TOTAL_MS=$(echo "scale=4; $TOTAL_VAL / 1000" | bc)
    else
        TOTAL_MS="${RAW_TOTAL%ms}"
    fi

    echo "$GIT_MS $LANG_MS $TOTAL_MS" >> "$TIMINGS_FILE"
    printf "."
done
echo ""

# Analyze with python
python3 -c "
import sys
import statistics

data = [line.split() for line in sys.stdin]
git_times = [float(row[0]) for row in data]
lang_times = [float(row[1]) for row in data]
total_times = [float(row[2]) for row in data]

def stats(name, times):
    print(f'\n--- {name} ---')
    print(f'Count: {len(times)}')
    print(f'Mean:  {statistics.mean(times):.4f} ms')
    print(f'Min:   {min(times):.4f} ms')
    print(f'Max:   {max(times):.4f} ms')
    print(f'Stdev: {statistics.stdev(times):.4f} ms')
    try:
        print(f'P95:   {sorted(times)[int(len(times)*0.95)]:.4f} ms')
    except:
        pass
    print(f'P99:   {sorted(times)[int(len(times)*0.99)]:.4f} ms')

stats('Git Status IPC', git_times)
stats('Language Detection', lang_times)
stats('Total Render Estimate', total_times)
" < "$TIMINGS_FILE"

rm "$TIMINGS_FILE"
