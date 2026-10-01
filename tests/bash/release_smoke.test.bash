#!/usr/bin/env bash
# tests/bash/release_smoke.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# scripts/smoke-release.sh executes the packaged binaries and fails on a
# broken one (#652).
#
# The release workflow's smoke job runs that script against the payload
# scripts/package-release.sh produced. This builds the same payload locally
# around the debug binaries (the other platforms' assets are stubs the
# packager only checksums) and:
#   (a) the smoke run passes: install.sh from the archive, both `--version`s,
#       a oneshot render, a responding agent, one prompt per shell
#   (b) an agent that answers `--version` correctly but fails everything
#       else (so install.sh's own functionality check cannot catch it) makes
#       the smoke run FAIL -- the assertion the workflow relies on

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command zsh
test_require_command python3
test_require_command tar
test_require_command zip

GPY_CLI="$ROOT/gpy-agent/target/debug/gpy"
GPY_AGENT="$ROOT/gpy-agent/target/debug/gpy-agent"
if [ ! -x "$GPY_CLI" ] || [ ! -x "$GPY_AGENT" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
fi

case "$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)" in
    linux-x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
    linux-aarch64 | linux-arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
    darwin-x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
    darwin-arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
    *) test_skip "no release asset for $(uname -s) $(uname -m)" ;;
esac

VERSION="v$("$GPY_AGENT" --version | awk '{print $2}')"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

work="$(mktemp -d "$(shell_e2e_test_root)/release-smoke.XXXXXX" 2>/dev/null || mktemp -d "${TMPDIR:-/tmp}/release-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# `build_payload DIR AGENT_FILE` -- a packaged payload in DIR whose native
# agent asset is AGENT_FILE; every other asset is a stub.
build_payload() {
    local out="$1" agent_file="$2" artifacts="$1/artifacts" asset name
    mkdir -p "$artifacts"
    for asset in gpy-agent-linux-x86_64 gpy-agent-linux-aarch64 gpy-agent-macos-x86_64 gpy-agent-macos-aarch64 \
        gpy-agent-windows-x86_64.exe gpy-linux-x86_64 gpy-linux-aarch64 gpy-macos-x86_64 gpy-macos-aarch64 gpy-windows-x86_64.exe; do
        mkdir -p "$artifacts/$asset"
        case "$asset" in
            gpy-agent-*.exe) name=gpy-agent.exe ;;
            gpy-agent-*) name=gpy-agent ;;
            *.exe) name=gpy.exe ;;
            *) name=gpy ;;
        esac
        if [ "$asset" = "$AGENT_ASSET" ]; then
            cp "$agent_file" "$artifacts/$asset/$name"
        elif [ "$asset" = "$CLI_ASSET" ]; then
            cp "$GPY_CLI" "$artifacts/$asset/$name"
        else
            printf '#!/bin/sh\nexit 0\n' >"$artifacts/$asset/$name"
        fi
        chmod +x "$artifacts/$asset/$name"
    done
    (cd "$out" && "$ROOT/scripts/package-release.sh" --version "$VERSION" --artifacts "$artifacts" --out . --no-sbom) \
        >"$out/package.log" 2>&1 || {
        echo "FAIL: package-release.sh failed:"
        tail -n 20 "$out/package.log"
        return 1
    }
}

echo "--- (a) the real binaries pass the smoke run ---"
mkdir -p "$work/good"
build_payload "$work/good" "$GPY_AGENT" || exit 1
if "$ROOT/scripts/smoke-release.sh" --package "$work/good" --version "$VERSION" >"$work/good/smoke.log" 2>&1; then
    pass "smoke-release.sh passes on the real binaries"
else
    fail "smoke-release.sh failed on the real binaries:"
    tail -n 30 "$work/good/smoke.log"
fi
for line in "gpy-agent --version reports" "oneshot git renders" "Running and Responding" \
    "fish renders a prompt" "zsh renders a prompt" "bash renders a prompt"; do
    if grep -q "PASS: .*$line" "$work/good/smoke.log"; then
        pass "the smoke run asserted: $line"
    else
        fail "the smoke run did not assert: $line"
    fi
done

echo "--- (b) a broken agent that still answers --version fails the smoke run ---"
mkdir -p "$work/broken"
broken="$work/broken-agent"
# shellcheck disable=SC2016  # $1 is for the stub itself
printf '#!/bin/sh\ncase "$1" in --version) echo "gpy-agent %s"; exit 0 ;; esac\nexit 1\n' "${VERSION#v}" >"$broken"
chmod +x "$broken"
build_payload "$work/broken" "$broken" || exit 1
if "$ROOT/scripts/smoke-release.sh" --package "$work/broken" --version "$VERSION" >"$work/broken/smoke.log" 2>&1; then
    fail "smoke-release.sh passed with an agent that fails every command but --version"
    tail -n 20 "$work/broken/smoke.log"
else
    pass "smoke-release.sh fails on the broken agent"
fi
if grep -q "^FAIL: gpy-agent oneshot git" "$work/broken/smoke.log"; then
    pass "the failure names the oneshot render"
else
    fail "expected a oneshot-render failure in the broken run:"
    tail -n 20 "$work/broken/smoke.log"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the release smoke run executes the packaged binaries and catches a broken one"
