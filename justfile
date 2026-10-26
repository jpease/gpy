# GPY development tasks

agent_dir := "gpy-agent"
scripts_dir := "scripts"

import? 'private.just'

# List recipes
default:
    @just --list

# Run the full quality checks
check:
    ./{{scripts_dir}}/quality-check.sh

# Run fast quality checks (skips slow audits and release builds)
check-fast:
    ./{{scripts_dir}}/quality-check.sh --fast

# Run quality checks for Rust only
check-rust:
    ./{{scripts_dir}}/quality-check.sh --rust-only

# Run quality checks for Fish only (lint: syntax + formatting)
check-fish:
    ./{{scripts_dir}}/quality-check.sh --fish-only

# Run shell checks only (skips Rust); optional comma-separated parts: checks,fish,bash,zsh
check-shell parts="":
    ./{{scripts_dir}}/quality-check.sh --shell-only {{parts}}

# Compile for Linux (clippy) and Windows (mingw) in Docker; needs Docker
check-platforms *args:
    ./{{scripts_dir}}/check-platforms.sh {{args}}

# Apply auto-fixes (formatting, clippy fixes)
fix:
    ./{{scripts_dir}}/quality-check.sh --fix

# Report known advisories in dependency manifests (report-only; not part of `check`)
audit:
    #!/usr/bin/env sh
    if ! command -v osv-scanner >/dev/null 2>&1; then
        echo "osv-scanner is not installed; skipping the dependency audit."
        echo "Install: https://google.github.io/osv-scanner/installation/"
        exit 0
    fi
    osv-scanner scan source -r .
    status=$?
    # osv-scanner exit codes: 0 clean, 1-126 findings, 128 nothing scannable.
    if [ "$status" -eq 0 ]; then
        echo "No known advisories in the scanned manifests."
    elif [ "$status" -ge 1 ] && [ "$status" -le 126 ]; then
        echo "Advisories reported above. Classify each one before remediating;"
        echo "this is not a gate."
    elif [ "$status" -eq 128 ]; then
        echo "No supported dependency manifest found; nothing to audit."
    else
        echo "osv-scanner exited $status without completing the scan." >&2
    fi
    exit 0

# Run clippy over every target and feature (policy: gpy-agent/Cargo.toml [lints])
lint:
    ./{{scripts_dir}}/check-active-toolchain.sh
    cd {{agent_dir}} && cargo clippy --all-targets --all-features

# Line coverage for gpy-agent (lcov.info + summary); needs cargo-llvm-cov
coverage:
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov nextest --lcov --output-path lcov.info
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov report --summary-only

# Format the codebase (Rust)
format:
    cd {{agent_dir}} && cargo fmt

# Check formatting without modifying files
fmt-check:
    cd {{agent_dir}} && cargo fmt --check

# Run all tests (Rust agent + Shell integrations)
test: test-rust test-fish

# Run Rust unit and integration tests
test-rust:
    cd {{agent_dir}} && cargo nextest run

# Run Fish integration tests
test-fish:
    ./{{scripts_dir}}/test_fish.sh

# Enforce performance budgets: Rust micro-benchmarks, then prompt latency and agent memory
bench-ci:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0
    ./{{scripts_dir}}/bench.sh --ci || fail=1
    ./{{scripts_dir}}/ci-bench.sh --ci || fail=1
    exit $fail

# Compare canary benchmarks against an upstream commit on the same machine
perf-canary *args:
    ./{{scripts_dir}}/perf-canary.sh {{args}}

# GPY vs Starship comparison on pinned repos and tool versions; run on an idle machine (benchmarks/compare-starship/README.md)
bench-compare *args:
    ./benchmarks/compare-starship/run.sh {{args}}

# Build the agent in debug mode
build:
    cd {{agent_dir}} && cargo build

# Build the agent in release mode
build-release:
    cd {{agent_dir}} && cargo build --release

# Build release binaries for all supported platforms
build-all-platforms:
    ./{{scripts_dir}}/build-release-binaries.sh

# Clean build artifacts and caches
clean:
    cd {{agent_dir}} && cargo clean
    rm -rf bin/
    rm -rf target/

# Install GPY from this checkout (builds the agent)
install:
    fish install-dev.fish

# Run an unreleased build in an isolated shell (flags: see scripts/README.md)
dev *args:
    ./scripts/gpy-dev.sh {{args}}

# Run Rust micro-benchmarks (args passed to cargo bench)
bench-rust *args:
    cd {{agent_dir}} && cargo bench {{args}}

# Record the feature-walkthrough GIF into assets/demo.gif (see demo/README.md)
demo:
    ./demo/setup.fish

# Install development git hooks
install-hooks:
    ./{{scripts_dir}}/install-hooks.sh

# Uninstall development git hooks
uninstall-hooks:
    ./{{scripts_dir}}/install-hooks.sh --uninstall
