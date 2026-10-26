#!/usr/bin/env bash
# tests/bash/install_checksum_verification.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install.sh must refuse a bin/<asset> not matching its shipped checksum,
# including when the checksum is absent. Asserts:
#   (a) a package with correct checksums installs
#   (b) a missing sidecar aborts, installing nothing
#   (c) a tampered binary aborts, installing nothing
#   (d) a present-but-unverifiable gpy CLI aborts (absence is tolerated,
#       tampering is not)
#   (e) a failed verification leaves an existing installation untouched

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    test_skip "no SHA-256 tool available"
fi

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# Every asset the build matrix produces, so platform detection need not be reimplemented.
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

# Mirror install.sh case blocks exactly: it exits early on unsupported platforms.
case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
            aarch64 | arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    darwin)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
            arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    *)
        test_skip "install.sh does not support $(uname -s)"
        ;;
esac

# Binaries are shell scripts so the `--version` check and `gpy setup` / `gpy-agent start` pass.
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

run_install() {
    local pkg="$1" home="$2" log="$3" status=0
    rm -rf "$home"
    mkdir -p "$home"
    (
        cd "$pkg" || exit 99
        HOME="$home" XDG_CONFIG_HOME="$home/.config" GPY_SHELL=fish bash ./install.sh
    ) >"$log" 2>&1 || status=$?
    echo "$status"
}


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


echo "--- package with no checksum for the agent (must fail) ---"
PKG_NOSUM="$WORKDIR/pkg-nosum"
HOME_NOSUM="$WORKDIR/home-nosum"
build_package "$PKG_NOSUM"
rm -f "$PKG_NOSUM/bin/$AGENT_ASSET.sha256"
status="$(run_install "$PKG_NOSUM" "$HOME_NOSUM" "$WORKDIR/nosum.log")"

[[ "$status" -ne 0 ]] || fail "install.sh succeeded with no checksum for $AGENT_ASSET"
[[ ! -e "$HOME_NOSUM/.local/bin/gpy-agent" ]] ||
    fail "install.sh installed the agent despite having no checksum for it"


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

# A missing gpy asset is tolerated; a present-but-unverifiable one is not.

echo "--- package with an unverifiable CLI (must fail) ---"
PKG_CLI="$WORKDIR/pkg-cli"
HOME_CLI="$WORKDIR/home-cli"
build_package "$PKG_CLI"
printf '\n# injected\n' >>"$PKG_CLI/bin/$CLI_ASSET"
status="$(run_install "$PKG_CLI" "$HOME_CLI" "$WORKDIR/cli.log")"

[[ "$status" -ne 0 ]] || fail "install.sh succeeded with a tampered $CLI_ASSET"

echo "--- package with no CLI binary at all (must fail: the CLI runs gpy setup) ---"
PKG_NOCLI="$WORKDIR/pkg-nocli"
HOME_NOCLI="$WORKDIR/home-nocli"
build_package "$PKG_NOCLI"
rm -f "$PKG_NOCLI/bin/$CLI_ASSET" "$PKG_NOCLI/bin/$CLI_ASSET.sha256"
status="$(run_install "$PKG_NOCLI" "$HOME_NOCLI" "$WORKDIR/nocli.log")"

[[ "$status" -ne 0 ]] ||
    fail "install.sh succeeded without the CLI asset, which it needs to run gpy setup"
[[ ! -e "$HOME_NOCLI/.config/fish/config.fish" ]] ||
    fail "an rc file was written although the CLI was missing"


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
    HOME="$HOME_EXISTING" XDG_CONFIG_HOME="$HOME_EXISTING/.config" GPY_SHELL=fish bash ./install.sh
) >"$WORKDIR/existing.log" 2>&1 || status=$?

[[ "$status" -ne 0 ]] || fail "install.sh succeeded against a tampered package over an existing install"
if [[ "$(sha256_of "$HOME_EXISTING/.local/bin/gpy-agent")" != "$incumbent_sum" ]]; then
    fail "a failed verification replaced or modified the existing gpy-agent"
fi
if compgen -G "$HOME_EXISTING/.local/bin/gpy-agent.backup.*" >/dev/null; then
    fail "a failed verification left backup files behind; it should abort before touching anything"
fi

test_finish
