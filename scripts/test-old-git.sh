#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Run the untracked-cache guard tests (git::native::untracked_guard_tests) against Debian's older git
# in the pinned Rust container. Newer git (2.56) no longer replays a stale
# untracked-cache listing, so on a developer machine with it the untracked-cache
# guard's repair test skips; here it must run (CI=1 turns a skip into a failure).
#
# One module only: cargo test shares process-global statics across tests, which
# the nextest runs used elsewhere isolate.
#
# Usage: scripts/test-old-git.sh
# Exit codes: 0 pass, 1 a test failed, 2 Docker unreachable.
#
# The repository is mounted read-only; the named volume gpy-old-git-target
# keeps the target dir warm. The image tag must match gpy-agent/rust-toolchain.toml.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="rust:1.100.0-bookworm"

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    echo "test-old-git: docker is not available (install it or start the daemon)" >&2
    exit 2
fi

exec docker run --rm \
    -v "$ROOT":/src:ro \
    -v gpy-old-git-target:/target \
    -e CARGO_TARGET_DIR=/target -e CI=1 \
    -w /src/gpy-agent "$IMAGE" \
    sh -c 'git --version && cargo test --locked --lib git::native::untracked_guard_tests'
