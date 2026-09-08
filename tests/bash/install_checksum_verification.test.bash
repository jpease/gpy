#!/usr/bin/env bash
# tests/bash/install_checksum_verification.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #494: install.sh installed whatever bin/<asset> happened
# to contain. The archive ships checksums now, and the installer must refuse to
# install a binary that does not match one -- including when the checksum is
# absent entirely, since "the release published no verification metadata" and
# "someone stripped it" look identical from the installing machine.
#
# This runs install.sh for real against a synthetic package in a throwaway
# HOME, with shell-script stubs standing in for the binaries. It asserts:
#   (a) a package with correct checksums installs
#   (b) a missing sidecar aborts, installing nothing
#   (c) a tampered binary aborts, installing nothing
#   (d) a present-but-unverifiable gpy CLI aborts (absence is tolerated,
#       tampering is not)
#   (e) a failed verification leaves an existing installation untouched

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    echo "SKIP: no SHA-256 tool available"
    exit 0
fi

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Every asset name the build matrix produces, so the test does not have to
# reimplement install.sh's platform detection -- whichever pair this machine
# resolves to is present.
ASSETS=(
    gpy-agent-linux-x86_64
    gpy-agent-linux-aarch64
    gpy-agent-macos-x86_64
    gpy-agent-macos-aarch64
    gpy-agent-windows-x86_64.exe
    gpy-linux-x86_64
    gpy-linux-aarch64
    gpy-macos-x86_64
    gpy-macos-aarch64
    gpy-windows-x86_64.exe
)

# The asset install.sh will pick on this machine. install.sh exits before doing
# anything on an unsupported platform, so mirror its case blocks exactly.
case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
            aarch64 | arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
            *) echo "SKIP: unsupported architecture $(uname -m)"; exit 0 ;;
        esac
        ;;
    darwin)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
            arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
            *) echo "SKIP: unsupported architecture $(uname -m)"; exit 0 ;;
        esac
        ;;
    *)
        echo "SKIP: install.sh does not support $(uname -s)"
        exit 0
        ;;
esac

# Build a package that looks like an extracted gpy-release.tar.gz: bin/ with
# asset-named binaries plus their sidecars, and the fish/ tree install.sh
# copies. The "binaries" are shell scripts so that install.sh's `--version`
# functionality check passes and `gpy-agent init` / `start` succeed.
build_package() {
    local pkg="$1"
    rm -rf "$pkg"
    mkdir -p "$pkg/bin"
    cp -R "$ROOT/fish" "$pkg/fish"
    cp "$ROOT/install.sh" "$pkg/install.sh"
    chmod +x "$pkg/install.sh"

    local asset
    for asset in "${ASSETS[@]}"; do
        printf '#!/bin/sh\necho "%s 9.9.9"\n' "$asset" >"$pkg/bin/$asset"
        chmod +x "$pkg/bin/$asset"
        printf '%s  %s\n' "$(sha256_of "$pkg/bin/$asset")" "$asset" >"$pkg/bin/$asset.sha256"
    done
}

# Run install.sh from $pkg with a throwaway HOME. Echoes the exit status; the
# install output goes to a log the caller can grep.
run_install() {
    local pkg="$1" home="$2" log="$3" status=0
    rm -rf "$home"
    mkdir -p "$home"
    (
        cd "$pkg" || exit 99
        HOME="$home" XDG_CONFIG_HOME="$home/.config" bash ./install.sh
    ) >"$log" 2>&1 || status=$?
    echo "$status"
}

# --- (a) correct checksums install --------------------------------------------

echo "--- verified package installs ---"
PKG="$WORKDIR/pkg-good"
HOME_GOOD="$WORKDIR/home-good"
build_package "$PKG"
status="$(run_install "$PKG" "$HOME_GOOD" "$WORKDIR/good.log")"

if [[ "$status" -ne 0 ]]; then
    fail "install.sh exited $status on a package with valid checksums"
    sed -n '1,40p' "$WORKDIR/good.log"
fi
[[ -x "$HOME_GOOD/.local/bin/gpy-agent" ]] || fail "agent binary was not installed"
[[ -x "$HOME_GOOD/.local/bin/gpy" ]] || fail "CLI binary was not installed"
grep -q "Verified agent binary checksum" "$WORKDIR/good.log" ||
    fail "install.sh did not report verifying the agent checksum"
grep -q "Verified CLI binary checksum" "$WORKDIR/good.log" ||
    fail "install.sh did not report verifying the CLI checksum"

# --- (b) missing sidecar aborts -----------------------------------------------

echo "--- package with no checksum for the agent (must fail) ---"
PKG_NOSUM="$WORKDIR/pkg-nosum"
HOME_NOSUM="$WORKDIR/home-nosum"
build_package "$PKG_NOSUM"
rm -f "$PKG_NOSUM/bin/$AGENT_ASSET.sha256"
status="$(run_install "$PKG_NOSUM" "$HOME_NOSUM" "$WORKDIR/nosum.log")"

[[ "$status" -ne 0 ]] || fail "install.sh succeeded with no checksum for $AGENT_ASSET"
[[ ! -e "$HOME_NOSUM/.local/bin/gpy-agent" ]] ||
    fail "install.sh installed the agent despite having no checksum for it"

# --- (c) tampered binary aborts -----------------------------------------------

echo "--- package with a tampered agent binary (must fail) ---"
PKG_TAMPER="$WORKDIR/pkg-tamper"
HOME_TAMPER="$WORKDIR/home-tamper"
build_package "$PKG_TAMPER"
printf '\n# injected after the checksum was computed\n' >>"$PKG_TAMPER/bin/$AGENT_ASSET"
status="$(run_install "$PKG_TAMPER" "$HOME_TAMPER" "$WORKDIR/tamper.log")"

[[ "$status" -ne 0 ]] || fail "install.sh succeeded with a tampered $AGENT_ASSET"
[[ ! -e "$HOME_TAMPER/.local/bin/gpy-agent" ]] ||
    fail "install.sh installed a binary whose checksum did not match"
grep -q "checksum mismatch" "$WORKDIR/tamper.log" ||
    fail "install.sh did not report a checksum mismatch"

# --- (d) an unverifiable CLI aborts too ---------------------------------------
#
# A missing gpy asset is tolerated by design (#327): the prompt does not need
# the CLI. A gpy asset that is present but cannot be verified is not.

echo "--- package with an unverifiable CLI (must fail) ---"
PKG_CLI="$WORKDIR/pkg-cli"
HOME_CLI="$WORKDIR/home-cli"
build_package "$PKG_CLI"
printf '\n# injected\n' >>"$PKG_CLI/bin/$CLI_ASSET"
status="$(run_install "$PKG_CLI" "$HOME_CLI" "$WORKDIR/cli.log")"

[[ "$status" -ne 0 ]] || fail "install.sh succeeded with a tampered $CLI_ASSET"

echo "--- package with no CLI binary at all (must still succeed) ---"
PKG_NOCLI="$WORKDIR/pkg-nocli"
HOME_NOCLI="$WORKDIR/home-nocli"
build_package "$PKG_NOCLI"
rm -f "$PKG_NOCLI/bin/$CLI_ASSET" "$PKG_NOCLI/bin/$CLI_ASSET.sha256"
status="$(run_install "$PKG_NOCLI" "$HOME_NOCLI" "$WORKDIR/nocli.log")"

[[ "$status" -eq 0 ]] ||
    fail "install.sh exited $status when the optional CLI asset was absent (should degrade, not abort)"
[[ -x "$HOME_NOCLI/.local/bin/gpy-agent" ]] ||
    fail "agent was not installed when the optional CLI asset was absent"

# --- (e) a failed verification leaves an existing install untouched -----------

echo "--- failed verification must not disturb an existing install ---"
HOME_EXISTING="$WORKDIR/home-existing"
rm -rf "$HOME_EXISTING"
mkdir -p "$HOME_EXISTING/.local/bin"
printf '#!/bin/sh\necho "incumbent 1.0.0"\n' >"$HOME_EXISTING/.local/bin/gpy-agent"
chmod +x "$HOME_EXISTING/.local/bin/gpy-agent"
incumbent_sum="$(sha256_of "$HOME_EXISTING/.local/bin/gpy-agent")"

status=0
(
    cd "$PKG_TAMPER" || exit 99
    HOME="$HOME_EXISTING" XDG_CONFIG_HOME="$HOME_EXISTING/.config" bash ./install.sh
) >"$WORKDIR/existing.log" 2>&1 || status=$?

[[ "$status" -ne 0 ]] || fail "install.sh succeeded against a tampered package over an existing install"
if [[ "$(sha256_of "$HOME_EXISTING/.local/bin/gpy-agent")" != "$incumbent_sum" ]]; then
    fail "a failed verification replaced or modified the existing gpy-agent"
fi
if compgen -G "$HOME_EXISTING/.local/bin/gpy-agent.backup.*" >/dev/null; then
    fail "a failed verification left backup files behind; it should abort before touching anything"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
