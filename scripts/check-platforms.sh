#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Compile gpy-agent for Linux (clippy) and Windows (mingw cargo check) in a
# Docker container, so cfg-gated breakage shows up before CI does.
#
# Usage: scripts/check-platforms.sh [--worktree | <rev>]
#   <rev>        commit to check (default HEAD); streamed with `git archive`
#   --worktree   check HEAD plus uncommitted changes to tracked files
#                (via `git stash create`; untracked files are not included)
#
# Exit codes: 0 all checks pass, 1 a check failed, 2 Docker unreachable or
# bad usage.
#
# Caches: the image gpy-platforms:<toolchain> (mingw-w64, the Windows target,
# clippy) is built once; named volumes gpy-platforms-target and
# gpy-platforms-cargo keep the target dir and cargo registry warm.
set -euo pipefail

usage() {
    sed -n '7,13s/^# \{0,1\}//p' "$0" >&2
    exit 2
}

rev=HEAD
case "${1:-}" in
    -h | --help) usage ;;
    --worktree)
        rev=$(git stash create)
        rev=${rev:-HEAD}
        ;;
    "") ;;
    -*) usage ;;
    *) rev=$1 ;;
esac
[[ $# -le 1 ]] || usage

repo_root=$(git rev-parse --show-toplevel)
git -C "$repo_root" rev-parse --verify --quiet "${rev}^{commit}" >/dev/null || {
    echo "check-platforms: unknown revision: $rev" >&2
    exit 2
}

if ! docker info >/dev/null 2>&1; then
    echo "check-platforms: Docker is not reachable (is Docker running? DOCKER_HOST=${DOCKER_HOST:-unset})" >&2
    exit 2
fi

toolchain=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$repo_root/gpy-agent/rust-toolchain.toml")
[[ -n $toolchain ]] || {
    echo "check-platforms: no channel in gpy-agent/rust-toolchain.toml" >&2
    exit 2
}
image="gpy-platforms:$toolchain"

if ! docker image inspect "$image" >/dev/null 2>&1; then
    echo "check-platforms: building $image (one-time)" >&2
    docker build -q -t "$image" - >/dev/null <<EOF
FROM rust:$toolchain
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends mingw-w64 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup toolchain install $toolchain --profile minimal \
        --component clippy --target x86_64-pc-windows-gnu
EOF
fi

echo "check-platforms: checking $(git -C "$repo_root" rev-parse --short "$rev") with Rust $toolchain" >&2

# shellcheck disable=SC2016
# Expanded inside the container, not here.
inner='
set -u
mkdir /w && tar -x -C /w
cd /w/gpy-agent
unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER
export CARGO_TARGET_DIR=/target RUSTUP_TOOLCHAIN="$TOOLCHAIN" CARGO_TERM_COLOR=always
failed=()
run() {
    echo "==> $*"
    "$@" || failed+=("$*")
}
run cargo clippy --all-targets
run cargo clippy --lib --bins
win=(cargo check --target x86_64-pc-windows-gnu)
run "${win[@]}" --lib
run "${win[@]}" --locked
run "${win[@]}" --profile test --tests
if ((${#failed[@]})); then
    echo "check-platforms: FAILED:"
    printf "  %s\n" "${failed[@]}"
    exit 1
fi
echo "check-platforms: all platform checks passed"
'

git -C "$repo_root" archive "$rev" | docker run --rm -i \
    -e TOOLCHAIN="$toolchain" \
    -v gpy-platforms-target:/target \
    -v gpy-platforms-cargo:/usr/local/cargo/registry \
    "$image" bash -c "$inner"
