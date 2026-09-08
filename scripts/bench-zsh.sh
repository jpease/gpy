#!/usr/bin/env bash
# Benchmark Zsh prompt rendering performance

set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Zsh Prompt Performance Benchmark ==="
echo

# Start agent if not running
if ! gpy-agent status &>/dev/null; then
    gpy-agent start
    sleep 0.5
fi

# Run benchmark
zsh -c '
zmodload zsh/datetime
source zsh/gpy.zsh
__enabled_segments=(status directory git clock duration language)

echo "Benchmarking 100 prompt renders..."
total_time=0

for i in {1..100}; do
    start=$((EPOCHREALTIME * 1000000))
    PROMPT=$(__gpy_render_prompt 0)
    end=$((EPOCHREALTIME * 1000000))
    elapsed=$((end - start))
    total_time=$((total_time + elapsed))
done

avg_us=$((total_time / 100))
avg_ms=$((avg_us / 1000))

echo "Average render time: ${avg_ms}ms (${avg_us}µs)"

if [[ $avg_ms -gt 5 ]]; then
    echo "WARN: Slower than 5ms target"
else
    echo "PASS: Within performance target"
fi
'
