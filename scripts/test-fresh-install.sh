#!/usr/bin/env bash
# scripts/test-fresh-install.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build and run the clean-container fresh-install check (#652): a stock
# ubuntu:24.04 image with an empty $HOME installs GPY from an archive built
# from this checkout and renders one prompt in fish, zsh and bash. See
# tests/docker/Dockerfile.fresh-install for what is asserted.
#
# Usage: scripts/test-fresh-install.sh [--no-cache]
# Needs a running docker daemon; exits 2 (not 0) without one, so a gate that
# includes this step cannot pass by having no docker.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    echo "test-fresh-install: docker is not available (install it or start the daemon)" >&2
    exit 2
fi

version="v$(grep -m1 '^version = ' gpy-agent/Cargo.toml | sed 's/version = "\(.*\)"/\1/')"
image="gpy-fresh-install:local"
build_args=()
for arg in "$@"; do
    case "$arg" in
        --no-cache) build_args+=(--no-cache) ;;
        *) echo "test-fresh-install: unknown argument: $arg" >&2; exit 2 ;;
    esac
done

echo "==> Building $image (release build of $version inside the container)"
docker build "${build_args[@]}" -f tests/docker/Dockerfile.fresh-install -t "$image" .

echo "==> Fresh install and first prompts in a clean container"
docker run --rm -e "GPY_RELEASE_VERSION=$version" "$image"
