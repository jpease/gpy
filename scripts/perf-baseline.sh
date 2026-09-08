#!/usr/bin/env bash
# Performance baseline script for GPY
#
# This script runs all performance benchmarks and helps establish performance baselines.
# It's designed to be run manually when establishing or verifying baselines.
#
# Usage:
#   ./scripts/perf-baseline.sh              # Run all benchmarks
#   ./scripts/perf-baseline.sh --rust       # Run only Rust benchmarks
#   ./scripts/perf-baseline.sh --shell      # Run only shell benchmarks
#   ./scripts/perf-baseline.sh --compare    # Compare to existing baseline

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE_FILE="$PROJECT_ROOT/tests/performance-baselines.json"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

usage() {
    cat <<EOF
Performance Baseline Tool

Usage: $0 [OPTIONS]

OPTIONS:
    --rust          Run only Rust formatter benchmarks
    --shell         Run only shell integration benchmarks
    --compare       Compare current results to baseline
    --save          Save current results as new baseline
    --help          Show this help message

EXAMPLES:
    # Run all benchmarks
    $0

    # Run only formatter benchmarks
    $0 --rust

    # Run benchmarks and save as baseline
    $0 --save

    # Compare current performance to baseline
    $0 --compare

For more information, see docs/dev/performance/performance-baselines.md
EOF
}

log_info() {
    echo -e "${BLUE}[INFO]${NC} $*"
}

log_success() {
    echo -e "${GREEN}[✓]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[⚠]${NC} $*"
}

log_error() {
    echo -e "${RED}[✗]${NC} $*"
}

run_rust_benchmarks() {
    log_info "Running Rust formatter benchmarks..."

    cd "$PROJECT_ROOT/gpy-agent"

    if ! command -v cargo >/dev/null 2>&1; then
        log_error "cargo not found. Please install Rust toolchain."
        return 1
    fi

    # Run benchmarks
    cargo bench --bench formatter_bench

    log_success "Rust benchmarks complete"
    log_info "HTML report saved to: gpy-agent/target/criterion/report/index.html"
}

run_shell_benchmarks() {
    log_info "Running shell integration benchmarks..."

    if ! command -v hyperfine >/dev/null 2>&1; then
        log_warn "hyperfine not found. Install from: https://github.com/sharkdp/hyperfine"
        log_info "Skipping shell benchmarks"
        return 0
    fi

    if ! command -v fish >/dev/null 2>&1; then
        log_warn "fish shell not found. Skipping shell benchmarks"
        return 0
    fi

    cd "$PROJECT_ROOT"
    ./scripts/bench.sh

    log_success "Shell benchmarks complete"
}

compare_to_baseline() {
    if [[ ! -f "$BASELINE_FILE" ]]; then
        log_warn "No baseline file found at $BASELINE_FILE"
        log_info "Run with --save to create a baseline"
        return 0
    fi

    if ! command -v jq >/dev/null 2>&1; then
        log_error "jq is required for baseline comparison"
        log_info "Install jq: https://stedolan.github.io/jq/"
        return 1
    fi

    log_info "Comparing current results to baseline..."
    log_info "Baseline file: $BASELINE_FILE"
    echo

    # Extract regression thresholds
    MAJOR_THRESHOLD=$(jq -r '.regression_thresholds.major_regression_percent // 50' "$BASELINE_FILE")
    MINOR_THRESHOLD=$(jq -r '.regression_thresholds.minor_regression_percent // 20' "$BASELINE_FILE")
    VARIANCE=$(jq -r '.regression_thresholds.measurement_variance_percent // 15' "$BASELINE_FILE")

    log_info "Regression thresholds:"
    echo "  Major regression: >${MAJOR_THRESHOLD}%"
    echo "  Minor regression: >${MINOR_THRESHOLD}%"
    echo "  Variance tolerance: ±${VARIANCE}%"
    echo

    # Check budgets against baselines
    log_info "Budget compliance check:"
    echo

    HAS_VIOLATION=false

    # Compare each budget to its corresponding baseline
    while IFS= read -r budget_key; do
        MAX_MS=$(jq -r ".budgets.${budget_key}.max_ms // 0" "$BASELINE_FILE")
        DESCRIPTION=$(jq -r ".budgets.${budget_key}.description // \"\"" "$BASELINE_FILE")

        # Skip if budget not defined
        if [[ "$MAX_MS" == "0" ]] || [[ "$MAX_MS" == "null" ]]; then
            continue
        fi

        # Look for corresponding baseline measurement
        # Map budget keys to baseline keys (convention based)
        BASELINE_KEY=""
        case "$budget_key" in
            "ipc_latency")
                BASELINE_KEY="ipc_ping"
                ;;
            "git_status_cold")
                BASELINE_KEY="git_status_100_files"
                ;;
            "cache_write")
                BASELINE_KEY="cache_atomic_write"
                ;;
        esac

        if [[ -n "$BASELINE_KEY" ]]; then
            MEAN_MS=$(jq -r ".baselines.${BASELINE_KEY}.mean_ms // 0" "$BASELINE_FILE")

            if [[ "$MEAN_MS" != "0" ]] && [[ "$MEAN_MS" != "null" ]]; then
                # Calculate percentage of budget used
                PERCENT=$(echo "scale=1; ($MEAN_MS / $MAX_MS) * 100" | bc)

                # Determine status
                if (( $(echo "$MEAN_MS <= $MAX_MS" | bc -l) )); then
                    log_success "$budget_key: ${MEAN_MS}ms / ${MAX_MS}ms (${PERCENT}%) ✓"
                else
                    log_error "$budget_key: ${MEAN_MS}ms / ${MAX_MS}ms (${PERCENT}%) EXCEEDS BUDGET"
                    HAS_VIOLATION=true
                fi
                echo "  └─ $DESCRIPTION"
            else
                log_warn "$budget_key: No baseline measurement available"
                echo "  └─ $DESCRIPTION"
            fi
        else
            log_info "$budget_key: ${MAX_MS}ms budget (no baseline to compare)"
            echo "  └─ $DESCRIPTION"
        fi
        echo
    done < <(jq -r '.budgets | keys[]' "$BASELINE_FILE")

    # Summary
    echo
    if [[ "$HAS_VIOLATION" == "true" ]]; then
        log_error "⚠️  Some metrics exceed their budgets"
        return 1
    else
        log_success "✓ All measured metrics within budgets"
    fi

    echo
    log_info "To update baselines:"
    log_info "  1. Run benchmarks: ./scripts/perf-baseline.sh"
log_info "  3. See docs/dev/performance/performance-baselines.md for guidance"

    return 0
}

save_baseline() {
    log_info "Saving current results as baseline..."

    # Create baseline template
    TIMESTAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    COMMIT=$(git rev-parse HEAD 2>/dev/null || echo "unknown")

    cat > "$BASELINE_FILE" <<EOF
{
  "timestamp": "$TIMESTAMP",
  "commit": "$COMMIT",
  "note": "Baseline established with git branch truncation feature",
  "benchmarks": {
    "formatter": {
      "note": "Run 'cargo bench --bench formatter_bench' to populate these values",
      "git_status_ansi_short_branch": {
        "mean_ns": 0,
        "std_dev_ns": 0
      },
      "git_status_ansi_long_branch": {
        "mean_ns": 0,
        "std_dev_ns": 0
      },
      "git_status_ansi_truncated_branch": {
        "mean_ns": 0,
        "std_dev_ns": 0
      },
      "language_ansi_single": {
        "mean_ns": 0,
        "std_dev_ns": 0
      }
    },
    "ipc": {
      "note": "Measured via shell benchmarks",
      "roundtrip_latency_ms": 0,
      "std_dev_ms": 0
    }
  },
  "budgets": {
    "ipc_latency": {
      "max_ms": 15,
      "description": "IPC roundtrip must complete within 15ms"
    },
    "git_status_cold": {
      "max_ms": 750,
      "description": "Cold git status within 750ms"
    },
    "cache_write": {
      "max_ms": 25,
      "description": "Cache operations within 25ms"
    },
    "memory_usage": {
      "max_mb": 50,
      "description": "Agent memory footprint under 50MB"
    }
  },
  "regression_thresholds": {
    "major_regression_percent": 50,
    "measurement_variance_percent": 15
  }
}
EOF

    log_success "Baseline template saved to $BASELINE_FILE"
    log_info "Edit this file and populate with actual benchmark results"
    log_info "See docs/dev/performance/performance-baselines.md for details"
}

# Parse arguments
RUN_RUST=true
RUN_SHELL=true
COMPARE=false
SAVE=false

if [[ $# -eq 0 ]]; then
    # No args, run everything
    RUN_RUST=true
    RUN_SHELL=true
else
    # Parse specific options
    RUN_RUST=false
    RUN_SHELL=false

    while [[ $# -gt 0 ]]; do
        case $1 in
            --rust)
                RUN_RUST=true
                shift
                ;;
            --shell)
                RUN_SHELL=true
                shift
                ;;
            --compare)
                COMPARE=true
                shift
                ;;
            --save)
                SAVE=true
                shift
                ;;
            --help)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done
fi

# Main execution
log_info "GPY Performance Baseline Tool"
echo

if [[ "$SAVE" == "true" ]]; then
    save_baseline
    exit 0
fi

if [[ "$COMPARE" == "true" ]]; then
    compare_to_baseline
    exit 0
fi

# Run benchmarks
if [[ "$RUN_RUST" == "true" ]]; then
    run_rust_benchmarks
    echo
fi

if [[ "$RUN_SHELL" == "true" ]]; then
    run_shell_benchmarks
    echo
fi

# Summary
log_success "Benchmarks complete!"
echo
log_info "Next steps:"
log_info "  1. Review results in gpy-agent/target/criterion/report/index.html"
log_info "  2. Update performance-baselines.json if needed"
log_info "  3. See docs/performance-baselines.md for guidance"
echo
