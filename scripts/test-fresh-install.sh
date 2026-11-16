#!/usr/bin/env bash
# scripts/test-fresh-install.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Install GPY in a clean ubuntu container and render one prompt per shell (see
# tests/docker/Dockerfile.fresh-install), then run each scenario in
# tests/docker/scenarios/ in a fresh container of the same image: upgrades
# from real published releases, setup idempotency, and the repository-code
# guarantees of SECURITY.md. Prints PASS or FAIL per check and fails when any
# check fails; a scenario that could not download a published asset is named
# as such, apart from a failed assertion.
#
# Usage: scripts/test-fresh-install.sh [--no-cache]
# Exits 2 without a docker daemon, so a gate cannot pass by having no docker.

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
docker build ${build_args[@]+"${build_args[@]}"} --build-arg "GPY_RELEASE_VERSION=$version" \
    -f tests/docker/Dockerfile.fresh-install -t "$image" .

results=()
failed=0

# `check NAME CMD...` -- run CMD in a fresh container and record its verdict.
# Exit 3 is tests/docker/scenario-lib.sh's "a published asset could not be
# downloaded"; any other non-zero exit is a failed assertion.
check() {
    local name="$1" rc=0 started=$SECONDS verdict
    shift
    echo
    echo "==> $name"
    docker run --rm -e "GPY_RELEASE_VERSION=$version" "$image" "$@" || rc=$?
    case "$rc" in
        0) verdict="PASS" ;;
        3) verdict="FAIL (network/asset unavailable: a published release could not be downloaded; nothing was tested)" ;;
        *) verdict="FAIL (test assertion failed, exit $rc)" ;;
    esac
    (( rc == 0 )) || failed=$((failed + 1))
    results+=("$(printf '%-28s %4ss  %s' "$name" "$((SECONDS - started))" "$verdict")")
    if (( rc != 0 )); then
        echo "test-fresh-install: $name: $verdict" >&2
    fi
}

check fresh-install-smoke
for scenario in tests/docker/scenarios/*.sh; do
    name="$(basename "$scenario" .sh)"
    check "$name" bash "/opt/scenarios/$name.sh"
done

echo
echo "==> Results"
printf '%s\n' "${results[@]}"
if (( failed > 0 )); then
    echo "test-fresh-install: $failed of ${#results[@]} check(s) failed" >&2
    exit 1
fi
echo "test-fresh-install: all ${#results[@]} checks passed"
