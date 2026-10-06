# GPY - The ultra-fast, ultra-efficient shell prompt
# Root justfile for development tasks

# Set project-wide variables
agent_dir := "gpy-agent"
scripts_dir := "scripts"

# Machine-local recipes (not committed). Absent in a fresh clone; `import?`
# skips it silently rather than erroring.
import? 'private.just'

# Default recipe: show help
default:
    @just --list

# --- Code Quality ---

# Run comprehensive quality checks
check:
    ./{{scripts_dir}}/quality-check.sh
    [ -f .raven/git-hooks/lib/with-verified-cache.sh ] && sh .raven/git-hooks/lib/with-verified-cache.sh check true || true

# Run fast quality checks (skips slow audits and release builds)
check-fast:
    ./{{scripts_dir}}/quality-check.sh --fast

# Run quality checks for Rust only
check-rust:
    ./{{scripts_dir}}/quality-check.sh --rust-only

# Run quality checks for Fish only (lint: syntax + formatting)
check-fish:
    ./{{scripts_dir}}/quality-check.sh --fish-only

# Run shell checks only (Fish/Bash/Zsh integration suites; skips Rust toolchain)
check-shell:
    ./{{scripts_dir}}/quality-check.sh --shell-only

# Apply auto-fixes (formatting, clippy fixes)
fix:
    ./{{scripts_dir}}/quality-check.sh --fix

# Report known advisories in this project's dependency manifests.
#
# Deliberately NOT a dependency of `check`. Every other recipe here is a
# function of the working tree; an audit is a function of the tree AND of what
# the world published overnight, so as a gate it would turn an unchanged commit
# red for reasons no one can fix in that commit. A gate is also binary, while an
# advisory has to be classified first -- see the Advisory Triage section of the
# `raven-dependency-update` skill. Report-only: never fails the shell.
audit:
    #!/usr/bin/env sh
    if ! command -v osv-scanner >/dev/null 2>&1; then
        echo "osv-scanner is not installed; skipping the dependency audit."
        echo "Install: https://google.github.io/osv-scanner/installation/"
        exit 0
    fi
    osv-scanner scan source -r .
    status=$?
    # Documented exit codes: 0 clean, 1-126 result-related (findings),
    # 127 general error, 128 nothing scannable, 129-255 other errors.
    if [ "$status" -eq 0 ]; then
        echo "No known advisories in the scanned manifests."
    elif [ "$status" -ge 1 ] && [ "$status" -le 126 ]; then
        echo "Advisories reported above. Classify each one before remediating"
        echo "(see the raven-dependency-update skill); this is not a gate."
    elif [ "$status" -eq 128 ]; then
        echo "No supported dependency manifest found; nothing to audit."
    else
        echo "osv-scanner exited $status without completing the scan." >&2
    fi
    exit 0

# Run lint checks (strict clippy for the agent)
lint:
    ./{{scripts_dir}}/check-active-toolchain.sh
    moon run gpy-agent:clippy

# Line coverage for gpy-agent (#654): lcov.info in gpy-agent/ plus a summary
# table on stdout. Non-gating; the ubuntu pr-gate uploads the same file.
# Needs cargo-llvm-cov and the toolchain's llvm-tools-preview component.
coverage:
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov nextest --features test-support --lcov --output-path lcov.info
    cd {{agent_dir}} && RUSTC_WRAPPER="" cargo llvm-cov report --summary-only

# Run type checks / static analysis
typecheck:
    cd {{agent_dir}} && cargo check --all-targets

# Format the codebase (Rust)
format:
    cd {{agent_dir}} && cargo fmt

# Check formatting without modifying files
fmt-check:
    cd {{agent_dir}} && cargo fmt --check

# --- Testing ---

# Run all tests (Rust agent + Shell integrations)
test: test-rust test-fish

# Run Rust unit and integration tests
test-rust:
    moon run gpy-agent:test

# Run Fish integration tests
test-fish:
    ./{{scripts_dir}}/test_fish.sh

# --- Benchmarking ---

# Run end-to-end shell integration benchmarks (requires hyperfine)
bench-shell *args:
    ./{{scripts_dir}}/bench.sh {{args}}

# Run CI benchmarks with performance budget enforcement.
# Runs both the Rust micro-benchmark gate (bench.sh) and the end-to-end
# prompt-latency + agent-memory gate (ci-bench.sh). Both run even if the first
# fails, so all budget violations surface in a single invocation.
bench-ci:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0
    ./{{scripts_dir}}/bench.sh --ci || fail=1
    ./{{scripts_dir}}/ci-bench.sh --ci || fail=1
    exit $fail

# Run performance baseline comparison or establishment
bench-baseline *args:
    ./{{scripts_dir}}/perf-baseline.sh {{args}}

# Compare canary benchmarks against an upstream commit on the same machine
perf-canary *args:
    ./{{scripts_dir}}/perf-canary.sh {{args}}

# Run benchmarks against huge repositories (10k+ files)
bench-huge *args:
    ./{{scripts_dir}}/benchmark-huge-repos.sh {{args}}

# Measure performance variance over multiple runs
bench-variance *args:
    ./{{scripts_dir}}/variance-check.sh {{args}}

# Run Zsh-specific benchmarks
bench-zsh:
    ./{{scripts_dir}}/bench-zsh.sh

# --- Development ---

# Build the agent in debug mode
build:
    moon run gpy-agent:build

# Build the agent in release mode
build-release:
    cd {{agent_dir}} && cargo build --release

# Build release binaries for all supported platforms
build-all-platforms:
    ./{{scripts_dir}}/build-release-binaries.sh

# Install development git hooks
install-hooks:
    ./{{scripts_dir}}/install-hooks.sh

# Uninstall development git hooks
uninstall-hooks:
    ./{{scripts_dir}}/install-hooks.sh --uninstall

# Clean build artifacts and caches
clean:
    cd {{agent_dir}} && cargo clean
    rm -rf bin/
    rm -rf target/

# Clear the shell's language cache (Fish only)
clear-cache:
    fish {{scripts_dir}}/clear_cache.fish

# --- Installation ---

# Install GPY from this checkout (builds the agent; install.sh is archive-only)
install:
    fish install-dev.fish

# --- Rust Agent Specific (Proxies) ---

# Run clippy for the agent
clippy *args:
    just -f {{agent_dir}}/justfile clippy {{args}}

# Run development clippy for the agent
clippy-dev:
    just -f {{agent_dir}}/justfile clippy-dev

# Run clippy (fast local check). The lint policy (deny warnings, deny
# clippy::pedantic) lives in gpy-agent/Cargo.toml's [lints] table (#584) and
# applies here exactly as it does in pre-push's `check-rust` / CI — there is
# no longer a "less strict" local tier. The one real difference from
# pre-push is scope, not strictness: this skips --features test-support, so
# the #460 PTY test-support surface isn't compiled or linted here.
clippy-strict:
    ./{{scripts_dir}}/check-active-toolchain.sh
    cd {{agent_dir}} && cargo clippy --all-targets

# Run Rust micro-benchmarks for the agent
bench-rust *args:
    just -f {{agent_dir}}/justfile bench {{args}}

# --- Demo ---

# Record the feature-walkthrough GIF into assets/demo.gif (isolated fixture $HOME; see demo/README.md)
demo:
    ./demo/setup.fish

# --- Maintenance ---

# Install all development and quality tools
setup: install-hooks
    just -f {{agent_dir}}/justfile install-tools
