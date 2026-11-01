# GPY development tasks.
# Before pushing, run `just check`: the full gate, strict about missing tools like CI;
# the release build runs only with `GPY_GATE_RELEASE=1`. Recipes from a machine-local
# `private.just` (gitignored) are imported if present.

agent_dir := "gpy-agent"
scripts_dir := "scripts"

import? 'private.just'

# List recipes
default:
    @just --list

# The pre-push check: full gate, strict about missing tools (as CI)
[group('gate')]
check:
    GPY_GATE_STRICT=1 ./{{scripts_dir}}/quality-check.sh

# Run fast quality checks (skips slow audits and release builds)
[group('gate')]
check-fast:
    ./{{scripts_dir}}/quality-check.sh --fast

# Run quality checks for Rust only
[group('gate')]
check-rust:
    ./{{scripts_dir}}/quality-check.sh --rust-only

# Run quality checks for Fish only (lint: syntax + formatting)
[group('gate')]
check-fish:
    ./{{scripts_dir}}/quality-check.sh --fish-only

# Run shell checks only (skips Rust); optional comma-separated parts: checks,repo,fish,bash,zsh (bash1 or bash2 for half the Bash suite)
[group('gate')]
check-shell parts="":
    ./{{scripts_dir}}/quality-check.sh --shell-only {{parts}}

# Compile for Linux (clippy) and Windows (mingw) in Docker; needs Docker
[group('gate')]
check-platforms *args:
    ./{{scripts_dir}}/check-platforms.sh {{args}}

# Apply auto-fixes (formatting, clippy fixes)
[group('gate')]
fix:
    ./{{scripts_dir}}/quality-check.sh --fix

# Report known advisories (exit 0 even with findings; non-zero when the scan cannot run; not part of `check`)
[group('report')]
audit:
    #!/usr/bin/env sh
    if ! command -v osv-scanner >/dev/null 2>&1; then
        echo "osv-scanner is not installed; cannot run the dependency audit." >&2
        echo "Install: https://google.github.io/osv-scanner/installation/" >&2
        exit 1
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
        exit "$status"
    fi
    exit 0

# Run clippy exactly as the gate does (policy: gpy-agent/Cargo.toml [lints])
[group('gate')]
lint:
    ./{{scripts_dir}}/quality-check.sh --clippy

# Line coverage for gpy-agent (lcov.info + summary); needs cargo-llvm-cov
[group('test')]
coverage:
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov nextest --lcov --output-path lcov.info
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov report --summary-only

# Format the codebase (Rust)
[group('gate')]
format:
    cd {{agent_dir}} && cargo fmt

# Check formatting without modifying files
[group('gate')]
fmt-check:
    cd {{agent_dir}} && cargo fmt --check

# Run all tests (Rust agent + repo, Fish, Bash and Zsh suites)
[group('test')]
test: test-rust test-shell

# Run Rust unit and integration tests
[group('test')]
test-rust:
    cd {{agent_dir}} && cargo nextest run

# Run the repo, Fish, Bash and Zsh test suites
[group('test')]
test-shell:
    ./{{scripts_dir}}/quality-check.sh --shell-only repo,fish,bash,zsh

# Enforce performance budgets: Rust micro-benchmarks, then prompt latency and agent memory
[group('bench')]
bench-ci:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0
    ./{{scripts_dir}}/bench.sh --ci || fail=1
    ./{{scripts_dir}}/ci-bench.sh --ci || fail=1
    exit $fail

# Compare canary benchmarks against an upstream commit on the same machine
[group('bench')]
perf-canary *args:
    ./{{scripts_dir}}/perf-canary.sh {{args}}

# GPY vs Starship comparison on pinned repos and tool versions; run on an idle machine (benchmarks/compare-starship/README.md)
[group('bench')]
bench-compare *args:
    ./benchmarks/compare-starship/run.sh {{args}}

# Time `gpy-agent oneshot git` on pinned real repositories (benchmarks/real-repos/README.md)
[group('bench')]
bench-repos *args:
    ./benchmarks/real-repos/run.sh {{args}}

# Build the agent in debug mode
[group('build')]
build:
    cd {{agent_dir}} && cargo build

# Build the agent in release mode
[group('build')]
build-release:
    cd {{agent_dir}} && cargo build --release

# Build release binaries for all supported platforms
[group('build')]
build-all-platforms:
    ./{{scripts_dir}}/build-release-binaries.sh

# Clean build artifacts and caches
[group('build')]
clean:
    cd {{agent_dir}} && cargo clean
    rm -rf bin/

# Install this checkout for Fish only (builds the agent; needs fish). Bash/Zsh: see install-dev.fish --help
[group('dev')]
install-dev:
    fish install-dev.fish

# Run an unreleased build in an isolated shell (flags: see scripts/README.md)
[group('dev')]
dev *args:
    ./scripts/gpy-dev.sh {{args}}

# Run Rust micro-benchmarks (args passed to cargo bench)
[group('bench')]
bench-rust *args:
    cd {{agent_dir}} && cargo bench {{args}}

# Record the feature-walkthrough GIF into assets/demo.gif (see demo/README.md)
[group('dev')]
demo:
    ./demo/setup.fish

# Install development git hooks
[group('dev')]
install-hooks:
    ./{{scripts_dir}}/install-hooks.sh

# Uninstall development git hooks
[group('dev')]
uninstall-hooks:
    ./{{scripts_dir}}/install-hooks.sh --uninstall
